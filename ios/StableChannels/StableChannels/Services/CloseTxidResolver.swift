import Foundation

/// Polls `/tx/{fundingTxid}/outspend/{vout}` for a cooperatively-closed
/// channel's closing txid. HTTP/retry plumbing lives in
/// `ResilientEsploraClient`. `databaseService` is per-call so the struct
/// stays Sendable.
struct CloseTxidResolver {
    /// Shared callback for both `CloseTxidResolver` and `OnchainTxidResolver`.
    /// `workId` is opId for close, `"res-\(resolutionId)"` for onchain-receive
    /// (caller parses the prefix to recover the Int64).
    typealias OnTxidResolved = @MainActor (_ workId: String, _ txid: String) async -> Void

    struct Config {
        let maxAttempts: Int
        let backoffSeconds: [UInt64]
        let esploraTimeout: TimeInterval
        let wallClockBudgetSeconds: TimeInterval
        let chainURLs: [String]

        init(
            maxAttempts: Int = 5,
            backoffSeconds: [UInt64] = [1, 4, 16, 64, 256],
            esploraTimeout: TimeInterval = 5,
            wallClockBudgetSeconds: TimeInterval = 900,
            chainURLs: [String]
        ) {
            self.maxAttempts = maxAttempts
            self.backoffSeconds = backoffSeconds
            self.esploraTimeout = esploraTimeout
            self.wallClockBudgetSeconds = wallClockBudgetSeconds
            self.chainURLs = chainURLs
        }
    }

    private let client: ResilientEsploraClient
    private let onResolved: OnTxidResolved

    init(
        chainURLs: [String],
        onResolved: @escaping OnTxidResolved,
        urlSession: URLSession = .shared,
        config: Config? = nil
    ) {
        precondition(!chainURLs.isEmpty, "CloseTxidResolver requires at least one chain URL")
        self.onResolved = onResolved
        let cfg = config ?? Config(chainURLs: chainURLs)
        self.client = ResilientEsploraClient(
            urlSession: urlSession,
            config: .init(
                chainURLs: cfg.chainURLs,
                maxAttempts: cfg.maxAttempts,
                backoffSeconds: cfg.backoffSeconds,
                timeout: cfg.esploraTimeout,
                wallClockBudgetSeconds: cfg.wallClockBudgetSeconds
            )
        )
    }

    static func isValidTxid(_ s: String) -> Bool {
        ResilientEsploraClient.isValidTxid(s)
    }

    func resolve(opId: String, databaseService: DatabaseService) async {
        // Snapshot from DB on caller's actor; check happens before any Sendable hop.
        let snapshot: (String, UInt32)? = await MainActor.run {
            guard let op = databaseService.fetchPendingOperation(opId: opId),
                  op.status == "pending",
                  let fundingTxid = op.fundingOutpointTxid,
                  let vout = op.fundingOutpointVout
            else { return nil }
            return (fundingTxid, vout)
        }
        guard let (fundingTxid, vout) = snapshot else { return }

        let onResolved = self.onResolved
        let parser: ResilientEsploraClient.ResultParser<String> = { data in
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let spent = json["spent"] as? Bool
            else { return nil }
            guard spent, let txid = json["txid"] as? String,
                  ResilientEsploraClient.isValidTxid(txid)
            else { return nil }
            return txid
        }

        await client.run(
            endpointBuilder: { base in
                ["\(ResilientEsploraClient.trimSlash(base))/tx/\(fundingTxid)/outspend/\(vout)"]
            },
            resultParser: parser,
            onResolved: { txid in
                // databaseService is non-Sendable; hop to MainActor for the DB write.
                await MainActor.run {
                    databaseService.updatePendingOperation(
                        opId: opId,
                        closingTxid: txid,
                        status: "resolved"
                    )
                }
                await onResolved(opId, txid)
            }
        )
    }
}
