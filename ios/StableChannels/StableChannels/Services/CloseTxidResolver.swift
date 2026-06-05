import Foundation

/// Polls `/tx/{fundingTxid}/outspend/{vout}` for a cooperatively-closed
/// channel's closing txid. HTTP/retry plumbing lives in
/// `ResilientEsploraClient`. `databaseService` is per-call so the struct
/// stays Sendable.
struct CloseTxidResolver {
    struct Config {
        let maxAttempts: Int
        let backoffSeconds: [UInt64]
        let esploraTimeout: TimeInterval
        let chainURLs: [String]

        init(
            maxAttempts: Int = 5,
            backoffSeconds: [UInt64] = [1, 4, 16, 64, 256],
            esploraTimeout: TimeInterval = 5,
            chainURLs: [String]
        ) {
            self.maxAttempts = maxAttempts
            self.backoffSeconds = backoffSeconds
            self.esploraTimeout = esploraTimeout
            self.chainURLs = chainURLs
        }
    }

    private let client: ResilientEsploraClient
    private let onResolved: @Sendable (String, String) async -> Void

    init(
        chainURLs: [String],
        onResolved: @escaping @Sendable (String, String) async -> Void,
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
                timeout: cfg.esploraTimeout
            )
        )
    }

    static func isValidTxid(_ s: String) -> Bool {
        ResilientEsploraClient.isValidTxid(s)
    }

    func resolve(opId: String, databaseService: DatabaseService) async {
        guard let op = databaseService.fetchPendingOperation(opId: opId),
              op.status == "pending",
              let fundingTxid = op.fundingOutpointTxid,
              let vout = op.fundingOutpointVout
        else { return }

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
                databaseService.updatePendingOperation(
                    opId: opId,
                    closingTxid: txid,
                    status: "resolved"
                )
                await onResolved(opId, txid)
            }
        )
    }
}
