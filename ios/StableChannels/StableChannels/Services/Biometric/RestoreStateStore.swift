import Foundation

/// Encapsulates durable restore-state persistence (UserDefaults I/O) for the
/// wallet lifecycle layer.
///
/// Extracted from `WalletLifecycleManager` to separate infrastructure concern
/// (UserDefaults reads/writes for `restore_phase`, `recovered_restore_pending`)
/// from domain logic (lifecycle state machine decisions). This keeps
/// `WalletLifecycleManager` under the 300-line hard cap and makes the
/// UserDefaults dependency explicit and testable via dependency injection.
struct RestoreStateStore {
    private let appGroupIdentifier: String

    private static let restorePhaseKey = "restore_phase"
    private static let recoveredRestorePendingKey = "recovered_restore_pending"
    private static let legacyRestoreInProgressKey = "restore_in_progress"

    init(appGroupIdentifier: String = Constants.appGroupIdentifier) {
        self.appGroupIdentifier = appGroupIdentifier
    }

    // MARK: - Recovered Restore Pending

    func isRecoveredRestorePending() -> Bool {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        return ud?.bool(forKey: Self.recoveredRestorePendingKey) == true
    }

    func setRecoveredRestorePending(_ pending: Bool) {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        if pending {
            ud?.set(true, forKey: Self.recoveredRestorePendingKey)
        } else {
            ud?.removeObject(forKey: Self.recoveredRestorePendingKey)
        }
    }

    func clearRecoveredRestorePending() {
        setRecoveredRestorePending(false)
    }

    // MARK: - Restore Phase

    func getRestorePhase() -> RestorePhase? {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        if let raw = ud?.string(forKey: Self.restorePhaseKey), let phase = RestorePhase(rawValue: raw) {
            return phase
        }
        if ud?.bool(forKey: Self.legacyRestoreInProgressKey) == true {
            return .pendingValidation
        }
        return nil
    }

    func setRestorePhase(_ phase: RestorePhase) throws {
        guard let ud = UserDefaults(suiteName: appGroupIdentifier) else {
            throw WalletRestoreError.wipeFailed("UserDefaults app group is inaccessible")
        }
        ud.set(phase.rawValue, forKey: Self.restorePhaseKey)
        ud.set(true, forKey: Self.legacyRestoreInProgressKey)
        guard ud.string(forKey: Self.restorePhaseKey) == phase.rawValue else {
            throw WalletRestoreError.wipeFailed("Failed to persist restore phase marker")
        }
    }

    func clearRestorePhase() {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        ud?.removeObject(forKey: Self.restorePhaseKey)
        ud?.removeObject(forKey: Self.legacyRestoreInProgressKey)
    }
}
