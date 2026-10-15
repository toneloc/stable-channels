import Foundation

/// Error raised when node start is asked to run without any provider to try.
enum NodeStartupError: Error {
    case noAttempts
}

/// Starts the node across `NodeStartupPolicy` attempts and, after a transient failure, keeps
/// retrying with backoff. Owns the retry state so there is one place to pause it (background),
/// resume it (foreground) and reset it (success, stop, wipe).
@MainActor
@Observable
final class NodeStartupCoordinator {
    /// Number of automatic retries scheduled since the last reset.
    private(set) var retryAttempt = 0
    /// True while node start is being retried after a transient chain-source failure.
    var isRetrying: Bool { retryAttempt > 0 }

    @ObservationIgnored private var retryTask: Task<Void, Never>?
    @ObservationIgnored private let sleep: (Duration) async throws -> Void
    @ObservationIgnored private let log: (String, [String: Any]) -> Void

    init(
        sleep: @escaping (Duration) async throws -> Void = { try await Task.sleep(for: $0) },
        log: @escaping (String, [String: Any]) -> Void = { AuditService.log($0, data: $1) }
    ) {
        self.sleep = sleep
        self.log = log
    }

    /// Tries each attempt in order and returns the URL the node started against. Retryable
    /// errors move on to the next attempt; any other error propagates immediately. When every
    /// attempt fails, the first error is rethrown so callers see the preferred provider's failure.
    func start(
        _ attempts: [NodeStartupPolicy.Attempt],
        isRetryable: (Error) -> Bool,
        using startNode: (String) async throws -> Void
    ) async throws -> String {
        guard let primaryURL = attempts.first?.url else { throw NodeStartupError.noAttempts }
        var firstError: Error?
        var lastError: Error?

        for (index, attempt) in attempts.enumerated() {
            if attempt.delayBefore > .zero {
                try await sleep(attempt.delayBefore)
            }
            do {
                try await startNode(attempt.url)
                if index > 0 {
                    log("NODE_START_FAILOVER_SUCCESS", [
                        "primary": primaryURL,
                        "using": attempt.url,
                        "attempt": "\(index)"
                    ])
                }
                reset()
                return attempt.url
            } catch {
                guard isRetryable(error) else {
                    // Non-Esplora errors (database, storage, lock, entropy, already running)
                    // must propagate immediately without retrying.
                    throw error
                }
                if firstError == nil {
                    firstError = error
                    log("NODE_START_INITIAL_FAILED", [
                        "esploraURL": attempt.url,
                        "error": error.localizedDescription
                    ])
                }
                lastError = error
            }
        }

        log("NODE_START_FAILOVER_FAILED", [
            "primary": primaryURL,
            "attempts": "\(attempts.count)",
            "primary_error": firstError?.localizedDescription ?? "",
            "last_error": lastError?.localizedDescription ?? ""
        ])
        throw firstError ?? lastError ?? NodeStartupError.noAttempts
    }

    /// Schedules one retry after the policy backoff. A pending retry is not duplicated. The
    /// handle is cleared before `retry` runs so a failure inside it can schedule the next one.
    func scheduleRetry(maxDelay: Duration, reason: String, retry: @escaping @MainActor () async -> Void) {
        guard retryTask == nil else { return }
        let delay = NodeStartupPolicy.retryDelay(forAttempt: retryAttempt, maxDelay: maxDelay)
        retryAttempt += 1
        log("NODE_START_RETRY_SCHEDULED", [
            "attempt": "\(retryAttempt)",
            "delay_s": "\(delay.components.seconds)",
            "error": reason
        ])
        let sleep = sleep
        retryTask = Task { [weak self] in
            do { try await sleep(delay) } catch { return }
            guard let self, !Task.isCancelled else { return }
            self.retryTask = nil
            await retry()
        }
    }

    /// Stops a pending retry but keeps the backoff, so foreground can resume where it left off.
    func pause() {
        retryTask?.cancel()
        retryTask = nil
    }

    /// Stops a pending retry and forgets the backoff.
    func reset() {
        pause()
        retryAttempt = 0
    }
}
