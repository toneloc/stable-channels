import Foundation

/// Rules for starting the node against Esplora providers: which providers to try and in what
/// order, how long to wait between automatic retries, and what a failed start leads to.
enum NodeStartupPolicy {
    /// One node-start try against an Esplora provider.
    struct Attempt: Equatable {
        let url: String
        /// Pause before this try, so a briefly stalled provider gets a moment to recover.
        let delayBefore: Duration
    }

    /// What a failed node start leads to.
    enum FailureOutcome: Equatable {
        /// Show the offline page; the network monitor drives recovery.
        case offline
        /// Keep the syncing screen and retry with backoff.
        case retry
        /// Surface the error.
        case fail
    }

    /// The preferred provider, every other provider once, then the preferred provider again
    /// after `retryDelay`.
    static func attempts(preferred: String, providers: [String], retryDelay: Duration) -> [Attempt] {
        var seen: Set<String> = [preferred]
        let others = providers.filter { seen.insert($0).inserted }
        return ([preferred] + others).map { Attempt(url: $0, delayBefore: .zero) }
            + [Attempt(url: preferred, delayBefore: retryDelay)]
    }

    /// Backoff between automatic retries: 1s, 2s, 4s, ... doubling per attempt, capped at `maxDelay`.
    static func retryDelay(forAttempt attempt: Int, maxDelay: Duration) -> Duration {
        let exponent = max(attempt, 0)
        // 2^62 seconds is already far past any sane cap; shifting further would overflow Int.
        guard exponent < 62 else { return maxDelay }
        return min(.seconds(1 << exponent), maxDelay)
    }

    /// No network wins, so the user sees the offline page instead of retries that cannot succeed.
    /// Online, a transient chain-source failure retries even when its text looks like a network
    /// drop ("timed out"); other offline-looking errors go offline and the rest are surfaced.
    static func failureOutcome(isNetworkOffline: Bool, looksOffline: Bool, isRetryable: Bool) -> FailureOutcome {
        if isNetworkOffline { return .offline }
        if isRetryable { return .retry }
        return looksOffline ? .offline : .fail
    }

    /// Whether another automatic retry is allowed. Once the budget is spent the user gets the
    /// error screen with Try Again instead of a syncing screen that retries forever.
    static func hasRetriesLeft(retryAttempt: Int, maxRetries: Int) -> Bool {
        retryAttempt < maxRetries
    }
}
