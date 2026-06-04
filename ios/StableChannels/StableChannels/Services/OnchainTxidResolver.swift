import Foundation

/// Polls Esplora for the first txid hitting a receive address. On hit,
/// updates the DB row (`resolutionId`) and fires `onResolved`. Thin
/// policy wrapper over `ResilientEsploraClient`: paths + parser only.
struct OnchainTxidResolver {
    typealias OnResolved = @MainActor (Int64, String) async
        -> Void // (resolutionId, txid); never throws

    private let client: ResilientEsploraClient
    private let onResolved: OnResolved

    init(
        chainURLs: [String],
        onResolved: @escaping OnResolved,
        urlSession: URLSession = .shared,
        maxAttempts: Int = 8,
        backoffSeconds: [UInt64] = [2, 8, 30, 60, 120, 300, 600, 900],
        esploraTimeout: TimeInterval = 8,
        wallClockBudgetSeconds: TimeInterval = 420
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

    /// Poll for any tx hitting `address`. On first hit, update the DB
    /// row at `resolutionId` and fire `onResolved`. Returns silently
    /// on exhaustion, budget overrun, or cancellation.
    func resolve(resolutionId: Int64, address: String, databaseService: DatabaseService) async {
        let onResolved = self.onResolved
        let parser: ResilientEsploraClient.ResultParser<String> = { data in
            guard let arr = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
            else { return nil }
            if let first = arr.first,
               let txid = first["txid"] as? String,
               ResilientEsploraClient.isValidTxid(txid) {
                return txid
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
                if databaseService.updateOnchainReceiveResolution(id: resolutionId, txid: txid) {
                    await onResolved(resolutionId, txid)
                }
            }
        )
    }
}
