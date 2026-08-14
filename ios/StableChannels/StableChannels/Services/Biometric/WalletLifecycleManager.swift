import Foundation

enum StartupState {
    case ready
    case newWallet
    case seedOnlyMismatch
    case dbOnlyMismatch
}

final class WalletLifecycleManager {
    private let keychain: any MnemonicStorageProtocol
    private let userDataDir: URL
    private let appGroupIdentifier: String

    init(
        keychain: any MnemonicStorageProtocol = WalletKeychainService.shared,
        userDataDir: URL = Constants.userDataDir,
        appGroupIdentifier: String = Constants.appGroupIdentifier
    ) {
        self.keychain = keychain
        self.userDataDir = userDataDir
        self.appGroupIdentifier = appGroupIdentifier
    }

    /// Evaluates the 4 possible database and seed startup states
    func detectStartupState() -> StartupState {
        let seedPath = userDataDir.appendingPathComponent("keys_seed")
        let seedPhrasePath = userDataDir.appendingPathComponent("seed_phrase")
        let hasSeed = FileManager.default.fileExists(atPath: seedPath.path)
            || keychain.hasMnemonic()
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
    func runRecoveryIfNeeded(onWipePersistence: () -> Void) throws {
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        guard ud?.bool(forKey: "restore_in_progress") == true else { return }

        AuditService.log("RESTORE_INTERRUPTED_RECOVERY_START", data: [:])
        do {
            if let pendingMnemonic = try? keychain.loadPendingMnemonic(), !pendingMnemonic.isEmpty {
                // Complete recovery: Promote seed first, then wipe database
                try keychain.storeMnemonic(pendingMnemonic)
                onWipePersistence()
                try keychain.deletePendingMnemonic()
                ud?.removeObject(forKey: "restore_in_progress")
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_SUCCESS", data: [:])
            } else {
                ud?.removeObject(forKey: "restore_in_progress")
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_NO_PENDING", data: [:])
            }
        } catch {
            AuditService.log("RESTORE_INTERRUPTED_RECOVERY_FAILED", data: ["error": error.localizedDescription])
            throw error // Retain the restore_in_progress marker by propagating the throw!
        }
    }

    /// Executes the staged restore transaction safely.
    /// Wipes old active states only after the new seed is verified & promoted to active slot.
    func restoreMnemonic(
        _ mnemonic: String,
        onStopNode: () -> Void,
        onWipePersistence: () -> Void
    ) throws {
        let words = mnemonic.trimmingCharacters(in: .whitespacesAndNewlines)

        // 1. Store and verify pending seed (abort if write fails - active wallet is untouched)
        try keychain.storePendingMnemonic(words)

        // 2. Record restore marker
        let ud = UserDefaults(suiteName: appGroupIdentifier)
        ud?.set(true, forKey: "restore_in_progress")

        // 3. Promote pending seed to active slot BEFORE wiping the DB (fails closed, DB is untouched)
        do {
            try keychain.storeMnemonic(words)
        } catch {
            AuditService.log("RESTORE_PROMOTION_FAILED", data: ["error": error.localizedDescription])
            ud?.removeObject(forKey: "restore_in_progress")
            try? keychain.deletePendingMnemonic()
            throw error
        }

        // 4. Stop node and wipe database files (we are now safely committed to the new seed)
        onStopNode()
        onWipePersistence()

        // 5. Clean up pending seed and clear marker
        try? keychain.deletePendingMnemonic()
        ud?.removeObject(forKey: "restore_in_progress")
    }
}
