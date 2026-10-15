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
    /// The steps of one automatic retry, supplied by the caller. The coordinator runs them in
    /// order and stops between them once the retry is cancelled.
    struct RetryRun {
        /// Whether the retry should go ahead; may redirect the UI (for example to the offline page).
        let shouldRun: @MainActor () -> Bool
        /// Picks the provider to start against.
        let probe: @MainActor () async -> String
        /// Starts the node against the probed provider.
        let start: @MainActor (String) async -> Void
    }

    /// Number of automatic retries scheduled since the last reset.
    private(set) var retryAttempt = 0
    /// True while node start is being retried after a transient chain-source failure.
    var isRetrying: Bool { retryAttempt > 0 }

    @ObservationIgnored private var pendingRetry: Task<Void, Never>?
    @ObservationIgnored private var runningRetry: Task<Void, Never>?
    @ObservationIgnored private var retryGeneration = 0
    @ObservationIgnored private var runningGeneration: Int?
    /// True while a fired retry is still running (probing providers or starting the node).
    var hasRetryRunning: Bool { runningRetry != nil }
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

    /// Schedules one retry after the policy backoff. A pending retry is not duplicated. Once it
    /// fires, the retry moves from pending to running: it stays cancellable until it finishes, and
    /// the free pending slot lets a failure inside it schedule the next attempt.
    func scheduleRetry(maxDelay: Duration, reason: String, run: RetryRun) {
        guard pendingRetry == nil else { return }
        let delay = NodeStartupPolicy.retryDelay(forAttempt: retryAttempt, maxDelay: maxDelay)
        retryAttempt += 1
        log("NODE_START_RETRY_SCHEDULED", [
            "attempt": "\(retryAttempt)",
            "delay_s": "\(delay.components.seconds)",
            "error": reason
        ])
        let sleep = sleep
        retryGeneration &+= 1
        let generation = retryGeneration
        pendingRetry = Task { [weak self] in
            do { try await sleep(delay) } catch { return }
            guard let self, !Task.isCancelled else { return }
            self.runningRetry = self.pendingRetry
            self.runningGeneration = generation
            self.pendingRetry = nil
            await self.perform(run)
            if self.runningGeneration == generation {
                self.runningRetry = nil
                self.runningGeneration = nil
            }
        }
    }

    /// Resumes a paused retry on foreground unless one is still running, so returning to the app
    /// never starts a second node-start attempt alongside the first.
    func resumeIfIdle(maxDelay: Duration, reason: String, run: RetryRun) {
        guard runningRetry == nil else { return }
        scheduleRetry(maxDelay: maxDelay, reason: reason, run: run)
    }

    private func perform(_ run: RetryRun) async {
        guard run.shouldRun() else { return }
        let url = await run.probe()
        // Background-stop pauses the retry and may hand the wallet dir to the NSE during the
        // probe; a cancelled retry must not start the node.
        guard !Task.isCancelled else { return }
        await run.start(url)
    }

    /// Stops a pending or running retry but keeps the backoff, so foreground can resume where it
    /// left off. A running retry sees cancellation and must not start the node afterwards.
    func pause() {
        pendingRetry?.cancel()
        pendingRetry = nil
        runningRetry?.cancel()
        runningRetry = nil
        runningGeneration = nil
    }

    /// Stops any retry and forgets the backoff.
    func reset() {
        pause()
        retryAttempt = 0
    }
}
