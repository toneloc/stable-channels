import Foundation

/// Defines a contract for bounding sequential LDK event retries.
public protocol SyncRetryTracking: AnyObject, Sendable {
    func recordAttemptAndShouldGiveUp(key: String) -> Bool
    func clear(key: String)
}

/// Bounds how long AppState keeps retrying (and therefore blocking LDK's sequential event queue on)
/// the same unresolved signed trade-sync message. NodeService will not advance to the next LDK event
/// until the current one is acknowledged, so a trade result that can never commit (e.g. channel already closed)
/// would otherwise retry forever and silently block every event after it, including ChannelClosed.
public final class SyncRetryTracker: SyncRetryTracking, @unchecked Sendable {
    public static let defaultMaxAttempts: Int = 20
    public static let defaultMaxDurationSeconds: TimeInterval = 300 // 5 minutes

    private struct AttemptState {
        var count: Int
        var firstAttempt: Date
    }

    private let maxAttempts: Int
    private let maxDurationSeconds: TimeInterval
    private let now: @Sendable () -> Date
    private let lock = NSLock()
    private var attempts: [String: AttemptState] = [:]

    public init(
        maxAttempts: Int = SyncRetryTracker.defaultMaxAttempts,
        maxDurationSeconds: TimeInterval = SyncRetryTracker.defaultMaxDurationSeconds,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.maxAttempts = maxAttempts
        self.maxDurationSeconds = maxDurationSeconds
        self.now = now
    }

    /// Records an attempt for `key`. Returns `true` once `maxAttempts` or `maxDurationSeconds` is reached.
    public func recordAttemptAndShouldGiveUp(key: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }

        let currentTime = now()
        if var state = attempts[key] {
            state.count += 1
            let exceededDuration = currentTime.timeIntervalSince(state.firstAttempt) >= maxDurationSeconds
            let exceededAttempts = state.count > maxAttempts
            if exceededDuration || exceededAttempts {
                attempts.removeValue(forKey: key)
                return true
            }
            attempts[key] = state
            return false
        } else {
            attempts[key] = AttemptState(count: 1, firstAttempt: currentTime)
            return false
        }
    }

    /// Clears tracking for `key` once it resolves definitively (applied, invalid, or duplicate).
    public func clear(key: String) {
        lock.lock()
        defer { lock.unlock() }
        attempts.removeValue(forKey: key)
    }
}
