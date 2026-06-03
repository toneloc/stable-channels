import Foundation

/// Resolves the real close txid for a cooperatively-closed channel by
/// polling Esplora's `/outspend/{vout}` endpoint for the funding outpoint.
/// `databaseService` is passed per-call so the struct stays Sendable.
struct CloseTxidResolver {
    struct Config {
        let maxAttempts: Int
        let backoffSeconds: [UInt64]
        let esploraTimeout: TimeInterval
        let chainURLs: [String]
        let onResolved: @Sendable (String, String) async
            -> Void // (opId, closingTxid); never throws, caller handles its own errors

        init(
            maxAttempts: Int = 5,
            backoffSeconds: [UInt64] = [1, 4, 16, 64, 256],
            esploraTimeout: TimeInterval = 5,
            chainURLs: [String],
            onResolved: @escaping @Sendable (String, String) async -> Void
        ) {
            self.maxAttempts = maxAttempts
            self.backoffSeconds = backoffSeconds
            self.esploraTimeout = esploraTimeout
            self.chainURLs = chainURLs
            self.onResolved = onResolved
        }
    }

    private let urlSession: URLSession
    private let config: Config

    init(
        chainURLs: [String],
        onResolved: @escaping @Sendable (String, String) async -> Void,
        urlSession: URLSession = .shared,
        config: Config? = nil
    ) {
        precondition(!chainURLs.isEmpty, "CloseTxidResolver requires at least one chain URL")
        self.urlSession = urlSession
        self.config = config ?? Config(chainURLs: chainURLs, onResolved: onResolved)
    }

    static func isValidTxid(_ s: String) -> Bool {
        guard s.count == 64 else { return false }
        for c in s {
            switch c {
            case "0"..."9", "a"..."f": continue
            default: return false
            }
        }
        return true
    }

    func resolve(opId: String, databaseService: DatabaseService) async {
        guard let op = databaseService.fetchPendingOperation(opId: opId),
              op.status == "pending",
              let fundingTxid = op.fundingOutpointTxid,
              let vout = op.fundingOutpointVout
        else { return }

        for attempt in 0..<config.maxAttempts {
            if attempt > 0 {
                if Task.isCancelled { return }
                let sleepSeconds = config.backoffSeconds[
                    min(attempt - 1, config.backoffSeconds.count - 1)
                ]
                do {
                    try await Task.sleep(nanoseconds: sleepSeconds * 1_000_000_000)
                } catch {
                    return
                }
            }
            for chainURL in config.chainURLs {
                if Task.isCancelled { return }
                do {
                    let trimmedBase = chainURL.hasSuffix("/")
                        ? String(chainURL.dropLast())
                        : chainURL
                    guard !trimmedBase.isEmpty,
                          let url = URL(string: "\(trimmedBase)/tx/\(fundingTxid)/outspend/\(vout)")
                    else { continue }
                    var req = URLRequest(url: url, timeoutInterval: config.esploraTimeout)
                    req.httpMethod = "GET"
                    let (data, response) = try await urlSession.data(for: req)
                    guard let http = response as? HTTPURLResponse, http.statusCode == 200,
                          let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                          let spent = json["spent"] as? Bool
                    else { continue }

                    if spent, let spendingTxid = json["txid"] as? String,
                       Self.isValidTxid(spendingTxid) {
                        databaseService.updatePendingOperation(
                            opId: opId,
                            closingTxid: spendingTxid,
                            status: "resolved"
                        )
                        await config.onResolved(opId, spendingTxid)
                        return
                    }
                } catch {
                    continue
                }
            }
        }
    }
}
