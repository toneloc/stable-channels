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

    /// The notification extension's two tries: `attempts` without the delayed final retry, which
    /// its short run cannot wait out.
    static func extensionProviders(preferred: String, providers: [String]) -> [String] {
        attempts(preferred: preferred, providers: providers, retryDelay: .zero).prefix(2).map(\.url)
    }

    /// The provider a launch starts against: the primary when its probe answered, otherwise the fallback.
    static func launchProvider(primaryReachable: Bool, primary: String, fallback: String) -> String {
        primaryReachable ? primary : fallback
    }

    /// Wait before the first automatic retry; each later retry doubles it.
    static let baseRetryDelay: Duration = .seconds(1)

    /// Backoff between automatic retries: 1s, 2s, 4s, ... doubling per attempt, capped at `maxDelay`.
    static func retryDelay(forAttempt attempt: Int, maxDelay: Duration) -> Duration {
        let exponent = max(attempt, 0)
        // 2^62 seconds is already far past any sane cap; shifting further would overflow Int.
        guard exponent < 62 else { return maxDelay }
        return min(baseRetryDelay * (1 << exponent), maxDelay)
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

    /// What a scheduled retry does when it fires.
    enum RetryGate: Equatable {
        /// Probe providers and start the node.
        case run
        /// The node is already up; nothing to do.
        case skip
        /// Show the offline page; the network monitor drives recovery.
        case offline
    }

    /// A fired retry goes offline exactly when a retryable failure would, so the two never disagree.
    static func retryGate(isNodeRunning: Bool, isNetworkOffline: Bool) -> RetryGate {
        if isNodeRunning { return .skip }
        let outcome = failureOutcome(isNetworkOffline: isNetworkOffline, looksOffline: false, isRetryable: true)
        return outcome == .offline ? .offline : .run
    }

    /// Whether foreground resumes a retry paused by background: only while the syncing screen
    /// waits on it and the node is not up yet.
    static func shouldResumeRetry(isSyncing: Bool, isRetrying: Bool, isNodeRunning: Bool) -> Bool {
        isSyncing && isRetrying && !isNodeRunning
    }
}
