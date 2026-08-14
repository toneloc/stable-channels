import Foundation

enum StartupState: Equatable {
    case ready
    case newWallet
    case seedOnlyMismatch
    case dbOnlyMismatch
    case storageError(String)
}

enum RestorePhase: String, Codable {
    case pendingValidation
    case oldPersistenceWiped
}

enum WalletRestoreError: Error, LocalizedError, Equatable {
    case invalidMnemonic(String)
    case wipeFailed(String)

    var errorDescription: String? {
        switch self {
        case .invalidMnemonic(let msg): return msg
        case .wipeFailed(let msg): return msg
        }
    }
}

final class WalletLifecycleManager {
    private let keychain: any MnemonicStorageProtocol
    private let userDataDir: URL
    private let appGroupIdentifier: String

    private static let restorePhaseKey = "restore_phase"

    init(
        keychain: any MnemonicStorageProtocol = WalletKeychainService.shared,
        userDataDir: URL = Constants.userDataDir,
        appGroupIdentifier: String = Constants.appGroupIdentifier
    ) {
        self.keychain = keychain
        self.userDataDir = userDataDir
        self.appGroupIdentifier = appGroupIdentifier
    }

    /// Evaluates if a given mnemonic string matches valid BIP-39 word counts (12, 15, 18, 21, 24 words).
    static func isValidMnemonicFormat(_ mnemonic: String) -> Bool {
        let words = mnemonic.split(whereSeparator: \.isWhitespace)
        let validCounts = [12, 15, 18, 21, 24]
        return validCounts.contains(words.count)
    }

    /// Evaluates the possible database and seed startup states
    func detectStartupState() -> StartupState {
        let seedPath = userDataDir.appendingPathComponent("keys_seed")
        let seedPhrasePath = userDataDir.appendingPathComponent("seed_phrase")

        let hasKeychainSeed: Bool
        do {
            hasKeychainSeed = try keychain.hasMnemonic()
        } catch {
            AuditService.log("STARTUP_KEYCHAIN_ERROR", data: ["error": error.localizedDescription])
            return .storageError(error.localizedDescription)
        }

        let hasSeed = FileManager.default.fileExists(atPath: seedPath.path)
            || hasKeychainSeed
            || FileManager.default.fileExists(atPath: seedPhrasePath.path)

        let dbPath = userDataDir.appendingPathComponent("ldk_node_data.sqlite")
        let hasDb = FileManager.default.fileExists(atPath: dbPath.path)

        if hasSeed && hasDb {
            return .ready
        } else if !hasSeed && !hasDb {
            return .newWallet
        } else if hasSeed && !hasDb {
            return .seedOnlyMismatch
        } else {
            return .dbOnlyMismatch
        }
    }

    /// Runs recovery if an interrupted restore transaction is detected
    func runRecoveryIfNeeded(onWipePersistence: () throws -> Void) throws {
        guard let phase = getRestorePhase() else { return }

        AuditService.log("RESTORE_INTERRUPTED_RECOVERY_START", data: ["phase": phase.rawValue])
        do {
            let pendingMnemonic = try? keychain.loadPendingMnemonic()
            guard let pending = pendingMnemonic, !pending.isEmpty else {
                clearRestorePhase()
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_NO_PENDING", data: [:])
                return
            }

            switch phase {
            case .pendingValidation:
                // Old database may still exist: wipe persistence first, then advance
                try onWipePersistence()
                setRestorePhase(.oldPersistenceWiped)
                try keychain.storeMnemonic(pending)
                try keychain.deletePendingMnemonic()
                clearRestorePhase()

            case .oldPersistenceWiped:
                // Old database was already wiped: promote seed to active slot
                try keychain.storeMnemonic(pending)
                try keychain.deletePendingMnemonic()
                clearRestorePhase()
            }

            AuditService.log("RESTORE_INTERRUPTED_RECOVERY_SUCCESS", data: [:])
        } catch {
            AuditService.log("RESTORE_INTERRUPTED_RECOVERY_FAILED", data: ["error": error.localizedDescription])
            throw error // Retain the durable restore phase marker by propagating throw
        }
    }

    /// Executes the staged restore transaction safely.
    /// Order of operations:
    /// 1. Validate mnemonic format (fails before any mutation).
    /// 2. Store in pending Keychain slot.
    /// 3. Save durable restore phase (.pendingValidation).
    /// 4. Stop node.
    /// 5. Wipe old database & persistence files throwing.
    /// 6. Save durable restore phase (.oldPersistenceWiped).
    /// 7. Promote pending seed to active Keychain slot.
    /// 8. Delete pending seed & clear durable restore phase.
    func restoreMnemonic(
        _ mnemonic: String,
        onStopNode: () -> Void,
        onWipePersistence: () throws -> Void
    ) throws {
        let canonical = MnemonicMigrator.canonicalizeMnemonic(mnemonic)
        guard Self.isValidMnemonicFormat(canonical) else {
            AuditService.log("RESTORE_INVALID_MNEMONIC", data: [:])
            throw WalletRestoreError.invalidMnemonic("Invalid mnemonic word count. Expected 12 or 24 words.")
        }

        // 1. Store and verify pending seed (abort if write fails - active wallet is untouched)
        try keychain.storePendingMnemonic(canonical)

        // 2. Record durable restore phase
        setRestorePhase(.pendingValidation)

        // 3. Stop node and wipe old database/persistence (throwing)
        onStopNode()
        do {
            try onWipePersistence()
        } catch {
            AuditService.log("RESTORE_WIPE_FAILED", data: ["error": error.localizedDescription])
            throw error
        }

        // 4. Mark old persistence successfully wiped
        setRestorePhase(.oldPersistenceWiped)

        // 5. Promote pending seed to active slot
        do {
            try keychain.storeMnemonic(canonical)
        } catch {
            AuditService.log("RESTORE_PROMOTION_FAILED", data: ["error": error.localizedDescription])
            throw error
        }

        // 6. Clean up pending seed and clear restore phase
        try? keychain.deletePendingMnemonic()
        clearRestorePhase()
    }

    // MARK: - Durable State Helpers

    private func getRestorePhase() -> RestorePhase? {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        if let raw = ud?.string(forKey: Self.restorePhaseKey), let phase = RestorePhase(rawValue: raw) {
            return phase
        }
        if ud?.bool(forKey: "restore_in_progress") == true {
            return .pendingValidation
        }
        return nil
    }

    private func setRestorePhase(_ phase: RestorePhase) {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        ud?.set(phase.rawValue, forKey: Self.restorePhaseKey)
        ud?.set(true, forKey: "restore_in_progress")
    }

    private func clearRestorePhase() {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        ud?.removeObject(forKey: Self.restorePhaseKey)
        ud?.removeObject(forKey: "restore_in_progress")
    }
}
