import Foundation

/// Esplora poller. Wraps the `/tx/.../outspend/...`, `/address/.../txs/...`
/// calls shared by `CloseTxidResolver` and `OnchainTxidResolver`.
/// Owns: chain-URL fallback, backoff with jitter, wall-clock budget, body
/// cap, cancellation, and the txid format check. Caller supplies:
///   - `endpointBuilder`: paths to try on a chain, in order. `[]` skips.
///   - `resultParser`: 200/JSON body -> resolved value, or `nil` to retry.
///   - `onResolved`: fires once on success.
struct ResilientEsploraClient {
    struct Config {
        let chainURLs: [String]
        let maxAttempts: Int
        let backoffSeconds: [UInt64]
        let timeout: TimeInterval
        /// Total wall-clock cap across all attempts. `run` returns and
        /// fires `onExhausted` if the budget is exceeded.
        let wallClockBudgetSeconds: TimeInterval
        /// Fires once after the attempt loop falls through without a hit.
        let onExhausted: (@Sendable () async -> Void)?

        init(
            chainURLs: [String],
            maxAttempts: Int = 5,
            backoffSeconds: [UInt64] = [1, 4, 16, 64, 256],
            timeout: TimeInterval = 5,
            wallClockBudgetSeconds: TimeInterval = 900,
            onExhausted: (@Sendable () async -> Void)? = nil
        ) {
            precondition(!chainURLs.isEmpty, "ResilientEsploraClient needs at least one chain URL")
            self.chainURLs = chainURLs
            self.maxAttempts = maxAttempts
            self.backoffSeconds = backoffSeconds
            self.timeout = timeout
            self.wallClockBudgetSeconds = wallClockBudgetSeconds
            self.onExhausted = onExhausted
        }
    }

    /// Per-chain path list. Tried in order on one chain before falling
    /// back to the next chain in `chainURLs`. `[]` skips the chain.
    typealias EndpointBuilder = @Sendable (String) -> [String]

    /// 200/JSON body -> resolved value, or `nil` to try the next path.
    typealias ResultParser<T> = @Sendable (Data) throws -> T?

    let urlSession: URLSession
    let config: Config

    init(urlSession: URLSession = .shared, config: Config) {
        self.urlSession = urlSession
        self.config = config
    }

    /// 1 MB cap on response body. Long-history `/address/.../txs/chain`
    /// can blow past this; a hostile or buggy mempool could push hundreds
    /// of MB into memory otherwise.
    private static let maxBodyBytes = 1_048_576

    /// Run the poll loop. Calls `onResolved` once on success, or returns
    /// (and fires `onExhausted`) after `maxAttempts` or the wall-clock
    /// budget is exhausted. Honors `Task.isCancelled` between attempts
    /// and chain URLs.
    ///
    /// Exit semantics:
    /// - `onResolved` fired: clean success, function returns.
    /// - `Task.isCancelled`: silent return. `onExhausted` is NOT fired — the
    ///   caller (e.g. `StaggeredTaskLauncher` replacing a stale task) is
    ///   responsible for any cleanup, and treating cancellation as
    ///   "exhaustion" would double-fire side effects.
    /// - Budget or attempts exhausted: falls through to `onExhausted`.
    ///   The exhaustion path is the only branch that fires the callback,
    ///   so callers can use `onExhausted` as a definitive "we tried and
    ///   gave up" signal.
    func run<T: Sendable>(
        endpointBuilder: @escaping EndpointBuilder,
        resultParser: @escaping ResultParser<T>,
        onResolved: @escaping @Sendable (T) async -> Void
    ) async {
        let start = Date()
        for attempt in 0..<config.maxAttempts {
            if Task.isCancelled {
                return
            }
            if Date().timeIntervalSince(start) >= config.wallClockBudgetSeconds {
                break
            }
            if attempt > 0 {
                let base = config.backoffSeconds[
                    min(attempt - 1, config.backoffSeconds.count - 1)
                ]
                let sleepNs = Self.jitteredBackoff(base: base) * 1_000_000_000
                // Skip the sleep if it would blow the wall-clock budget.
                if Date().timeIntervalSince(start) + (Double(sleepNs) / 1_000_000_000)
                    >= config.wallClockBudgetSeconds {
                    break
                }
                do {
                    try await Task.sleep(nanoseconds: sleepNs)
                } catch {
                    return
                }
                if Task.isCancelled {
                    return
                }
                if Date().timeIntervalSince(start) >= config.wallClockBudgetSeconds {
                    break
                }
            }
            for base in config.chainURLs {
                if Task.isCancelled {
                    return
                }
                let paths = endpointBuilder(base)
                for path in paths {
                    if Task.isCancelled {
                        return
                    }
                    guard let data = await fetchJSON(path: path) else { continue }
                    do {
                        if let hit = try resultParser(data) {
                            Self.log("ESPLORA_RESOLVED", [
                                "path": path,
                                "attempt": "\(attempt)"
                            ])
                            await onResolved(hit)
                            return
                        }
                    } catch {
                        continue
                    }
                }
            }
        }
        // Fell through without a hit AND without cancellation: true exhaustion.
        // Cancellation paths above already returned without reaching this point.
        Self.log("ESPLORA_EXHAUSTED", [
            "chainURLs": "\(config.chainURLs)",
            "attempts": "\(config.maxAttempts)"
        ])
        await config.onExhausted?()
    }

    /// GET `path`. Returns the body on 200, nil on non-200, network
    /// error, body too large, or invalid URL.
    func fetchJSON(path: String) async -> Data? {
        guard let url = URL(string: path) else { return nil }
        var req = URLRequest(url: url, timeoutInterval: config.timeout)
        req.httpMethod = "GET"
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        do {
            let (data, response) = try await urlSession.data(for: req)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200
            else { return nil }
            guard data.count <= Self.maxBodyBytes else {
                Self.log("ESPLORA_BODY_TOO_LARGE", [
                    "path": path,
                    "size": "\(data.count)"
                ])
                return nil
            }
            return data
        } catch {
            return nil
        }
    }

    /// +/-25% multiplicative jitter on a base backoff. Prevents N wallet
    /// instances from hammering the public mempool in lockstep.
    /// Rounded so a 1s base doesn't truncate to 0 at the lower bound.
    static func jitteredBackoff(base: UInt64) -> UInt64 {
        let jitter = Double.random(in: 0.75...1.25)
        return UInt64((Double(base) * jitter).rounded())
    }

    /// Terse AuditService wrapper.
    private static func log(_ event: String, _ data: [String: String]) {
        AuditService.log(event, data: data)
    }

    /// 64 lowercase hex chars.
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

    /// Strip a trailing slash from a chain base URL.
    static func trimSlash(_ base: String) -> String {
        base.hasSuffix("/") ? String(base.dropLast()) : base
    }
}
