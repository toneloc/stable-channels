import Foundation

/// Rate limiting for iCloud backup restore attempts.
/// - Max 5 attempts before lockout
/// - 30s lockout after 3 consecutive failures
/// - Counters reset on successful attempt or app restart
@MainActor
final class RateLimitService {
    static let shared = RateLimitService()

    private init() {}

    private let maxAttempts = 5
    private let lockoutDuration: TimeInterval = 30
    private let lockoutThreshold = 3

    private var attempts: Int {
        get { UserDefaults.standard.integer(forKey: "backupRestoreAttempts") }
        set { UserDefaults.standard.set(newValue, forKey: "backupRestoreAttempts") }
    }

    private var lockedUntil: Date? {
        get { UserDefaults.standard.object(forKey: "backupRestoreLockedUntil") as? Date }
        set { UserDefaults.standard.set(newValue, forKey: "backupRestoreLockedUntil") }
    }

    var attemptsRemaining: Int {
        max(0, maxAttempts - attempts)
    }

    var isLocked: Bool {
        guard let until = lockedUntil else { return false }
        if Date() < until {
            return true
        }
        // Lockout expired - reset attempts for a fresh session
        attempts = 0
        lockedUntil = nil
        return false
    }

    var lockoutRemainingSeconds: Int {
        guard let until = lockedUntil else { return 0 }
        return max(0, Int(until.timeIntervalSinceNow))
    }

    /// Records a failed restore attempt. Triggers lockout if threshold reached.
    func recordFailedAttempt() {
        attempts += 1
        if attempts >= lockoutThreshold {
            lockedUntil = Date().addingTimeInterval(lockoutDuration)
        }
    }

    /// Records a successful restore attempt. Resets all counters.
    func recordSuccessfulAttempt() {
        attempts = 0
        lockedUntil = nil
    }

    /// Checks current rate limit status. Throws if locked.
    func checkRateLimit() throws {
        if isLocked {
            throw BackupError.rateLimitExceeded(remainingSeconds: lockoutRemainingSeconds)
        }
    }

    /// Resets all rate limit counters.
    func reset() {
        attempts = 0
        lockedUntil = nil
    }
}
