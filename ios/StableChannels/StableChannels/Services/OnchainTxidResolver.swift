import Foundation

/// Polls Esplora for an unrecorded incoming transaction matching a deposit. On hit,
/// updates the DB row (`resolutionId`) and fires `onResolved`. Thin
/// policy wrapper over `ResilientEsploraClient`: paths + parser only.
struct OnchainTxidResolver {
    private let client: ResilientEsploraClient
    private let onResolved: CloseTxidResolver.OnTxidResolved

    init(
        chainURLs: [String],
        onResolved: @escaping CloseTxidResolver.OnTxidResolved,
        urlSession: URLSession = .shared,
        maxAttempts: Int = 8,
        backoffSeconds: [UInt64] = [2, 8, 30, 60, 120, 300, 600, 900],
        esploraTimeout: TimeInterval = 8,
        wallClockBudgetSeconds: TimeInterval = 900
    ) {
        precondition(!chainURLs.isEmpty, "OnchainTxidResolver requires at least one chain URL")
        self.onResolved = onResolved
        self.client = ResilientEsploraClient(
            urlSession: urlSession,
            config: .init(
                chainURLs: chainURLs,
                maxAttempts: maxAttempts,
                backoffSeconds: backoffSeconds,
                timeout: esploraTimeout,
                wallClockBudgetSeconds: wallClockBudgetSeconds
            )
        )
    }

    /// Poll for an unrecorded tx paying the pending amount to `address`. On hit, update the DB
    /// row at `resolutionId` and fire `onResolved`. Returns silently
    /// on exhaustion, budget overrun, or cancellation.
    func resolve(resolutionId: Int64, address: String, databaseService: DatabaseService) async {
        // A persisted address can have years of history. Never attach an already-accounted
        // transaction to a new deposit, and fail closed if history cannot be read.
        guard let context = await MainActor.run(body: { () -> (Set<String>, Int64)? in
            guard let excluded = try? databaseService.onchainRepo.recordedReceiveTxids(),
                  let payment = databaseService.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resolutionId),
                  payment.amountMsat > 0, payment.amountMsat % 1000 == 0 else { return nil }
            return (excluded, payment.amountMsat / 1000)
        }) else { return }
        let onResolved = self.onResolved
        let workId = "res-\(resolutionId)"
        let parser: ResilientEsploraClient.ResultParser<String> = { data in
            guard let arr = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
            else { return nil }
            for transaction in arr {
                if let txid = transaction["txid"] as? String,
                   ResilientEsploraClient.isValidTxid(txid), !context.0.contains(txid),
                   let outputs = transaction["vout"] as? [[String: Any]] {
                    let received = outputs.filter { $0["scriptpubkey_address"] as? String == address }
                        .compactMap { $0["value"] as? Int64 }
                    // Bind to an incoming output, not merely an address-history entry (which
                    // also includes spends). Non-matching aggregate deposits remain pending.
                    guard !received.isEmpty,
                          received.allSatisfy({ $0 > 0 && $0 <= context.1 }) else { continue }
                    var total: Int64 = 0
                    for value in received {
                        let sum = total.addingReportingOverflow(value)
                        if sum.overflow { return nil }
                        total = sum.partialValue
                    }
                    guard total == context.1 else { continue }
                    return txid
                }
            }
            return nil
        }

        await client.run(
            endpointBuilder: { base in
                let b = ResilientEsploraClient.trimSlash(base)
                return [
                    "\(b)/address/\(address)/txs/chain",
                    "\(b)/address/\(address)/txs/mempool"
                ]
            },
            resultParser: parser,
            onResolved: { txid in
                // databaseService is non-Sendable; hop to MainActor for the DB
                // call so the closure stays Sendable-safe.
                let updated = await MainActor.run {
                    databaseService.onchainRepo.updateOnchainReceiveResolution(id: resolutionId, txid: txid)
                }
                if updated {
                    await onResolved(workId, txid)
                } else {
                    // Update refused: row already resolved by an earlier resolver run,
                    // or the row was deleted. Skip onResolved to avoid clobbering the
                    // existing state. Log so a stuck UI is debuggable from audit trail.
                    await MainActor.run {
                        AuditService.log("ONCHAIN_RECEIVE_RES_UPDATE_SKIPPED", data: [
                            "resolution_id": "\(resolutionId)",
                            "txid": "\(txid)"
                        ])
                    }
                }
            }
        )
    }
}
