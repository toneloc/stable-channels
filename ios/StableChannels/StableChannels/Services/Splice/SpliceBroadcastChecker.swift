import Foundation
import OSLog

/// Outcome of asking Esplora whether it has ever heard of a txid.
public enum TxBroadcastStatus: Equatable, Sendable {
    case exists
    case notFound
    case inconclusive
}

/// Defines a contract for verifying whether a transaction made it on-chain/mempool.
public protocol SpliceBroadcastChecking: Sendable {
    func checkStatus(txid: String, endpointURLs: [String]) async -> TxBroadcastStatus
}

/// Checks whether Esplora has ever heard of a txid (broadcast, mempool, or confirmed) — used to
/// tell a genuinely abandoned/never-broadcast splice tx apart from a stale failure event for a
/// splice that did make it on-chain.
///
/// The result is tri-state:
/// - `.exists`: Any endpoint returned a successful (2xx) response for the tx.
/// - `.notFound`: Every reachable endpoint returned an explicit 404 across all retries.
/// - `.inconclusive`: Timeouts, 5xx, or network errors where non-existence cannot be proven.
public final class SpliceBroadcastChecker: SpliceBroadcastChecking, Sendable {
    private let urlSession: URLSession
    private let retries: Int
    private let retryDelayNanoseconds: UInt64
    private let sleeper: @Sendable (UInt64) async throws -> Void

    private static let logger = Logger(subsystem: "com.stablechannels.app", category: "SpliceBroadcastChecker")

    public init(
        urlSession: URLSession = .shared,
        retries: Int = 3,
        retryDelaySeconds: TimeInterval = 2.0,
        sleeper: @escaping @Sendable (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) {
        self.urlSession = urlSession
        self.retries = retries
        self.retryDelayNanoseconds = UInt64(retryDelaySeconds * 1_000_000_000)
        self.sleeper = sleeper
    }

    public func checkStatus(txid: String, endpointURLs: [String]) async -> TxBroadcastStatus {
        let normalizedTxid = txid.components(separatedBy: ":").first ?? txid
        let urls = endpointURLs.filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        if urls.isEmpty { return .inconclusive }

        for attempt in 0..<retries {
            var allNotFound = true
            var anyReached = false

            for baseURL in urls {
                let trimmedBase = baseURL.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
                guard let url = URL(string: "\(trimmedBase)/tx/\(normalizedTxid)/status") else {
                    continue
                }

                var request = URLRequest(url: url)
                request.httpMethod = "GET"
                request.timeoutInterval = 5.0

                do {
                    let (_, response) = try await urlSession.data(for: request)
                    guard let http = response as? HTTPURLResponse else {
                        allNotFound = false
                        continue
                    }
                    anyReached = true
                    if (200...299).contains(http.statusCode) {
                        return .exists
                    }
                    if http.statusCode != 404 {
                        allNotFound = false
                    }
                } catch {
                    Self.logger
                        .warning(
                            "Existence check for \(normalizedTxid) failed on \(baseURL): \(error.localizedDescription)"
                        )
                    allNotFound = false
                }
            }

            if !anyReached || !allNotFound {
                return .inconclusive
            }

            if attempt < retries - 1 {
                do {
                    try await sleeper(retryDelayNanoseconds)
                } catch {
                    return .inconclusive
                }
            }
        }

        return .notFound
    }
}
