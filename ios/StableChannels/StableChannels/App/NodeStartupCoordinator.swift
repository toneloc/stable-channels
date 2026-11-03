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
        /// Stops a node that came up after the app was paused, handing the wallet dir back.
        let stop: @MainActor () -> Void
    }

    /// Whether a requested retry was scheduled or the retry budget is spent.
    enum RetrySchedule: Equatable {
        case scheduled
        /// The budget is spent and the backoff was reset; the caller surfaces the error.
        case exhausted
    }

    /// Number of automatic retries that started running since the last reset. A retry that
    /// background cancels before or while it runs is not counted, so it never spends the budget.
    private(set) var retryAttempt = 0
    /// True while node start is being retried after a transient chain-source failure.
    private(set) var isRetrying = false

    @ObservationIgnored private var pendingRetry: Task<Void, Never>?
    @ObservationIgnored private var runningRetry: Task<Void, Never>?
    @ObservationIgnored private var retryGeneration = 0
    @ObservationIgnored private var runningGeneration: Int?
    /// Set by background, cleared by foreground: a start that finishes while paused is stopped.
    @ObservationIgnored private(set) var isPaused = false
    /// True while a fired retry is still running (probing providers or starting the node).
    var hasRetryRunning: Bool { runningRetry != nil }
    @ObservationIgnored private let sleep: (Duration) async throws -> Void
    @ObservationIgnored private let log: (String, [String: Any]) -> Void
    @ObservationIgnored private let chainURLStore: ChainURLStore

    init(
        sleep: @escaping (Duration) async throws -> Void = { try await Task.sleep(for: $0) },
        log: @escaping (String, [String: Any]) -> Void = { AuditService.log($0, data: $1) },
        chainURLStore: ChainURLStore = UserDefaultsChainURLStore(
            defaults: UserDefaults(suiteName: Constants.appGroupIdentifier)
        )
    ) {
        self.sleep = sleep
        self.log = log
        self.chainURLStore = chainURLStore
    }

    /// Starts against `preferred`, failing over per `NodeStartupPolicy.attempts`, and records the
    /// provider that worked so the next start (app or notification extension) begins with it.
    func startWithFailover(
        preferred: String,
        providers: [String] = Constants.esploraChainURLs,
        retryDelay: Duration = Constants.nodeStartRetryDelay,
        isRetryable: (Error) -> Bool,
        using startNode: (String) async throws -> Void
    ) async throws -> String {
        let attempts = NodeStartupPolicy.attempts(preferred: preferred, providers: providers, retryDelay: retryDelay)
        let url = try await start(attempts, isRetryable: isRetryable, using: startNode)
        chainURLStore.save(url)
        return url
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
            // A paused retry must not run further LDK starts while the app is suspended.
            try Task.checkCancellation()
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
                clearBackoff()
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

    /// Schedules one retry after the policy backoff, or resets and reports `.exhausted` once
    /// `maxRetries` retries have run.
    @discardableResult
    func scheduleRetry(
        maxDelay: Duration,
        maxRetries: Int = Constants.nodeStartMaxRetries,
        reason: String,
        run: RetryRun
    ) -> RetrySchedule {
        guard NodeStartupPolicy.hasRetriesLeft(retryAttempt: retryAttempt, maxRetries: maxRetries) else {
            log("NODE_START_RETRIES_EXHAUSTED", [
                "attempts": "\(retryAttempt)",
                "error": reason
            ])
            reset()
            return .exhausted
        }
        arm(maxDelay: maxDelay, reason: reason, run: run)
        return .scheduled
    }

    /// After background cancelled a retry mid-start: if foreground already came back, the retry
    /// it armed may have been turned away while this one ran, so schedule the next one here.
    /// Returns nil while still paused; foreground resumes the retry then.
    func rescheduleIfResumed(
        maxDelay: Duration,
        maxRetries: Int = Constants.nodeStartMaxRetries,
        reason: String,
        run: RetryRun
    ) -> RetrySchedule? {
        guard !isPaused else { return nil }
        return scheduleRetry(maxDelay: maxDelay, maxRetries: maxRetries, reason: reason, run: run)
    }

    /// Arms one retry after the policy backoff. A pending retry is not duplicated. Once it
    /// fires, the retry moves from pending to running: it stays cancellable until it finishes, and
    /// the free pending slot lets a failure inside it schedule the next attempt.
    private func arm(maxDelay: Duration, reason: String, run: RetryRun) {
        guard pendingRetry == nil else { return }
        isRetrying = true
        let delay = NodeStartupPolicy.retryDelay(forAttempt: retryAttempt, maxDelay: maxDelay)
        log("NODE_START_RETRY_SCHEDULED", [
            "attempt": "\(retryAttempt + 1)",
            "delay_s": "\(delay.components.seconds)",
            "error": reason,
            "armed": isPaused ? "false" : "true"
        ])
        // A suspended app must not start the node; foreground arms the retry.
        guard !isPaused else { return }
        let sleep = sleep
        retryGeneration &+= 1
        let generation = retryGeneration
        pendingRetry = Task { [weak self] in
            do { try await sleep(delay) } catch { return }
            guard let self, !Task.isCancelled else { return }
            self.retryAttempt += 1
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
        resume()
        guard runningRetry == nil else { return }
        arm(maxDelay: maxDelay, reason: reason, run: run)
    }

    private func perform(_ run: RetryRun) async {
        guard run.shouldRun() else { return }
        let url = await run.probe()
        // Background-stop pauses the retry and may hand the wallet dir to the NSE during the
        // probe; a cancelled retry must not start the node.
        guard !Task.isCancelled else { return }
        await run.start(url)
        // LDK's start can't be cancelled, so a retry paused mid-start may still bring the node up
        // in a suspended app; stop it so the NSE gets the wallet dir.
        if isPaused { run.stop() }
    }

    /// Back in foreground: a start that finishes from now on keeps its node.
    func resume() {
        isPaused = false
    }

    /// Stops a pending or running retry but keeps the backoff, so foreground can resume where it
    /// left off. A running retry sees cancellation and must not start the node afterwards.
    func pause() {
        isPaused = true
        pendingRetry?.cancel()
        pendingRetry = nil
        if let runningRetry {
            // Foreground runs the interrupted retry again; it must not count twice.
            retryAttempt -= 1
            runningRetry.cancel()
        }
        runningRetry = nil
        runningGeneration = nil
    }

    /// Stops any retry and forgets the backoff.
    func reset() {
        pause()
        isPaused = false
        retryAttempt = 0
        isRetrying = false
    }

    /// Forgets the backoff after a successful start. A running retry may be the caller, so it is
    /// not cancelled, only forgotten: a later pause must neither cancel the wallet sync that
    /// follows the start nor refund an attempt the reset already cleared.
    private func clearBackoff() {
        pendingRetry?.cancel()
        pendingRetry = nil
        runningRetry = nil
        runningGeneration = nil
        retryAttempt = 0
        isRetrying = false
    }
}
