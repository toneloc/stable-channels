import Foundation

enum StartupState: Equatable {
    case ready
    case newWallet
    case seedOnlyMismatch
    case dbOnlyMismatch
    case seedStorageMismatch
    case storageError(String)
}

enum RestorePhase: String, Codable {
    case pendingValidation
    case oldPersistenceWiped
}

enum WalletRestoreError: Error, LocalizedError, Equatable {
    case invalidMnemonic(String)
    case wipeFailed(String)
    case recoveryFailed(String)

    var errorDescription: String? {
        switch self {
        case .invalidMnemonic(let msg): return msg
        case .wipeFailed(let msg): return msg
        case .recoveryFailed(let msg): return msg
        }
    }
}

final class WalletLifecycleManager {
    private let keychain: any MnemonicStorageProtocol
    private let userDataDir: URL
    private let restoreStateStore: RestoreStateStore
    private let validator: (String) async -> Bool

    init(
        keychain: any MnemonicStorageProtocol = WalletKeychainService.shared,
        userDataDir: URL = Constants.userDataDir,
        restoreStateStore: RestoreStateStore = RestoreStateStore(),
        validator: @escaping (String) async -> Bool
    ) {
        self.keychain = keychain
        self.userDataDir = userDataDir
        self.restoreStateStore = restoreStateStore
        self.validator = validator
    }

    /// Evaluates the possible database and seed startup states
    func detectStartupState() -> StartupState {
        let seedPath = userDataDir.appendingPathComponent("keys_seed")
        let seedPhrasePath = userDataDir.appendingPathComponent("seed_phrase")

        let hasKeychainSeed: Bool
        let keychainSeed: String?
        do {
            hasKeychainSeed = try keychain.hasMnemonic()
            keychainSeed = hasKeychainSeed ? try keychain.loadMnemonic() : nil
        } catch {
            AuditService.log("STARTUP_KEYCHAIN_ERROR", data: ["error": error.localizedDescription])
            return .storageError(error.localizedDescription)
        }

        // Detect seed storage mismatch between secure Keychain and plaintext seed_phrase
        if FileManager.default.fileExists(atPath: seedPhrasePath.path) {
            do {
                let plaintext = try String(contentsOfFile: seedPhrasePath.path, encoding: .utf8)
                if let kcSeed = keychainSeed {
                    let canonicalPlaintext = BIP39.canonicalize(plaintext)
                    let canonicalKeychain = BIP39.canonicalize(kcSeed)
                    if !canonicalPlaintext.isEmpty, canonicalPlaintext != canonicalKeychain {
                        AuditService.log("STARTUP_SEED_STORAGE_MISMATCH", data: [:])
                        return .seedStorageMismatch
                    }
                }
            } catch {
                AuditService.log("STARTUP_PLAINTEXT_READ_ERROR", data: ["error": error.localizedDescription])
                return .storageError("Failed to read seed backup: \(error.localizedDescription)")
            }
        }

        let hasSeed = FileManager.default.fileExists(atPath: seedPath.path)
            || hasKeychainSeed
            || FileManager.default.fileExists(atPath: seedPhrasePath.path)

        let dbPath = userDataDir.appendingPathComponent("ldk_node_data.sqlite")
        let hasDb = FileManager.default.fileExists(atPath: dbPath.path)

        if hasSeed && hasDb {
            restoreStateStore.clearRecoveredRestorePending()
            return .ready
        } else if !hasSeed && !hasDb {
            return .newWallet
        } else if hasSeed && !hasDb {
            if restoreStateStore.isRecoveredRestorePending() {
                return .ready
            }
            return .seedOnlyMismatch
        } else {
            return .dbOnlyMismatch
        }
    }

    /// Runs recovery if an interrupted restore transaction is detected.
    /// The pending Keychain slot is the authoritative signal: Keychain writes are
    /// synchronous and durable, while the UserDefaults phase marker can be lost to
    /// an unflushed cache on a hard kill. A pending seed without a marker is still
    /// evidence of an in-flight restore and must never be treated as a new wallet.
    /// Returns true if an interrupted restore seed was promoted to the active slot.
    @discardableResult
    func runRecoveryIfNeeded(onWipePersistence: () throws -> Void) throws -> Bool {
        guard let phase = restoreStateStore.getRestorePhase() else {
            return try recoverMarkerlessPendingIfNeeded()
        }

        AuditService.log("RESTORE_INTERRUPTED_RECOVERY_START", data: ["phase": phase.rawValue])
        do {
            let pending: String
            do {
                pending = try keychain.loadPendingMnemonic()
            } catch WalletKeychainError.keyNotFound {
                let hasActive = (try? keychain.hasMnemonic()) ?? false
                if !hasActive {
                    AuditService.log("RESTORE_INTERRUPTED_RECOVERY_NO_SEEDS", data: ["phase": phase.rawValue])
                    throw WalletRestoreError.recoveryFailed(
                        "Restore phase is active but neither pending nor active seed exists in secure storage."
                    )
                }
                if phase == .oldPersistenceWiped {
                    // Old persistence was wiped and active seed is present: promotion already succeeded
                    // before the crash, but the process terminated before or during phase cleanup.
                    // Mark recovered_restore_pending before clearing phase so detectStartupState
                    // classifies this as .ready rather than .seedOnlyMismatch.
                    restoreStateStore.setRecoveredRestorePending(true)
                    restoreStateStore.clearRestorePhase()
                    AuditService.log("RESTORE_INTERRUPTED_RECOVERY_COMPLETED_PROMOTION", data: [:])
                    return true
                }
                restoreStateStore.clearRestorePhase()
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_NO_PENDING", data: [:])
                return false
            } catch {
                AuditService.log(
                    "RESTORE_INTERRUPTED_RECOVERY_KEYCHAIN_FAILED",
                    data: ["error": error.localizedDescription]
                )
                throw error
            }

            guard !pending.isEmpty else {
                let hasActive = (try? keychain.hasMnemonic()) ?? false
                if !hasActive {
                    AuditService.log("RESTORE_INTERRUPTED_RECOVERY_NO_SEEDS", data: ["phase": phase.rawValue])
                    throw WalletRestoreError.recoveryFailed(
                        "Restore phase is active but pending seed is empty and no active seed exists."
                    )
                }
                if phase == .oldPersistenceWiped {
                    restoreStateStore.setRecoveredRestorePending(true)
                    restoreStateStore.clearRestorePhase()
                    AuditService.log("RESTORE_INTERRUPTED_RECOVERY_COMPLETED_PROMOTION", data: [:])
                    return true
                }
                restoreStateStore.clearRestorePhase()
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_NO_PENDING", data: [:])
                return false
            }

            switch phase {
            case .pendingValidation:
                // Old database may still exist: wipe persistence first, then advance
                try onWipePersistence()
                try restoreStateStore.setRestorePhase(.oldPersistenceWiped)
                try keychain.storeMnemonic(pending)
                restoreStateStore.setRecoveredRestorePending(true)
                do {
                    try keychain.deletePendingMnemonic()
                } catch {
                    AuditService.log("RESTORE_PENDING_DELETE_FAILED", data: ["error": error.localizedDescription])
                }
                restoreStateStore.clearRestorePhase()
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_SUCCESS", data: [:])
                return true

            case .oldPersistenceWiped:
                // Old database was already wiped: promote seed to active slot
                try keychain.storeMnemonic(pending)
                restoreStateStore.setRecoveredRestorePending(true)
                do {
                    try keychain.deletePendingMnemonic()
                } catch {
                    AuditService.log("RESTORE_PENDING_DELETE_FAILED", data: ["error": error.localizedDescription])
                }
                restoreStateStore.clearRestorePhase()
                AuditService.log("RESTORE_INTERRUPTED_RECOVERY_SUCCESS", data: [:])
                return true
            }
        } catch {
            AuditService.log("RESTORE_INTERRUPTED_RECOVERY_FAILED", data: ["error": error.localizedDescription])
            throw error // Retain the durable restore phase marker by propagating throw
        }
    }

    /// Executes the staged restore transaction safely.
    /// Order of operations:
    /// 1. Validate BIP-39 mnemonic (fails before any mutation).
    /// 2. Store in pending Keychain slot.
    /// 3. Save durable restore phase (.pendingValidation).
    /// 4. Stop node.
    /// 5. Wipe old database & persistence files throwing.
    /// 6. Save durable restore phase (.oldPersistenceWiped).
    /// 7. Promote pending seed to active Keychain slot.
    /// 8. Mark recovered restore pending.
    /// 9. Delete pending seed & clear durable restore phase.
    func restoreMnemonic(
        _ mnemonic: String,
        onStopNode: () -> Void,
        onWipePersistence: () throws -> Void
    ) async throws {
        let canonical = BIP39.canonicalize(mnemonic)
        // The validator builds a full node to derive an identity — run it off the
        // main actor. Its failure can mean a bad checksum OR an environment error,
        // so the message must not claim the phrase itself is wrong.
        guard await validator(canonical) else {
            AuditService.log("RESTORE_INVALID_MNEMONIC", data: [:])
            throw WalletRestoreError.invalidMnemonic(
                "The recovery phrase could not be validated. Check each word and try again."
            )
        }

        // 1. Store and verify pending seed (abort if write fails - active wallet is untouched)
        try keychain.storePendingMnemonic(canonical)

        // 2. Record durable restore phase
        try restoreStateStore.setRestorePhase(.pendingValidation)

        // 3. Stop node and wipe old database/persistence (throwing)
        onStopNode()
        do {
            try onWipePersistence()
        } catch {
            AuditService.log("RESTORE_WIPE_FAILED", data: ["error": error.localizedDescription])
            throw error
        }

        // 4. Mark old persistence successfully wiped
        try restoreStateStore.setRestorePhase(.oldPersistenceWiped)

        // 5. Promote pending seed to active slot
        do {
            try keychain.storeMnemonic(canonical)
        } catch {
            AuditService.log("RESTORE_PROMOTION_FAILED", data: ["error": error.localizedDescription])
            throw error
        }

        // 6. Record recovered restore pending before deleting pending evidence and phase
        restoreStateStore.setRecoveredRestorePending(true)

        // 7. Clean up pending seed and clear restore phase
        do {
            try keychain.deletePendingMnemonic()
        } catch {
            AuditService.log("RESTORE_PENDING_DELETE_FAILED", data: ["error": error.localizedDescription])
        }
        restoreStateStore.clearRestorePhase()
    }

    /// Reconstructs the restore state when the phase marker was lost but a pending
    /// seed survives in the Keychain. Returns true if a pending seed was promoted.
    private func recoverMarkerlessPendingIfNeeded() throws -> Bool {
        let pending: String
        do {
            pending = try keychain.loadPendingMnemonic()
        } catch WalletKeychainError.keyNotFound {
            return false
        }
        guard !pending.isEmpty else { return false }

        // Fail closed on operational Keychain errors: promoting over a live wallet
        // that merely could not be read would destroy the wrong identity.
        let hasActive = try keychain.hasMnemonic()
        if hasActive {
            // The active seed survived, so the staged restore never reached the
            // wipe — the pending copy is abandoned staging. Remove it.
            AuditService.log("RESTORE_MARKERLESS_PENDING_CLEARED", data: [:])
            try? keychain.deletePendingMnemonic()
            return false
        }

        // "No active Keychain seed" does NOT prove the wipe completed: a legacy
        // wallet's identity lives in keys_seed/seed_phrase with its channel
        // database, and never had a Keychain entry at all. Promoting the pending
        // replacement over surviving legacy artifacts would open the old channel
        // database under a different identity. Only promote when nothing of the
        // old wallet remains; otherwise fail closed and preserve the evidence —
        // the legacy wallet keeps working, and the pending slot stays for the
        // user's next explicit restore.
        let legacyArtifacts = ["keys_seed", "seed_phrase", "ldk_node_data.sqlite"]
            .filter { name in
                FileManager.default.fileExists(
                    atPath: userDataDir.appendingPathComponent(name).path
                )
            }
        guard legacyArtifacts.isEmpty else {
            AuditService.log(
                "RESTORE_MARKERLESS_PENDING_BLOCKED_BY_LEGACY",
                data: ["artifacts": legacyArtifacts.joined(separator: ",")]
            )
            return false
        }

        // No active seed, no legacy artifacts, but a verified pending seed exists:
        // the wipe ran and the marker was lost. Promote the pending seed rather
        // than letting startup read this as a brand-new wallet and orphan the
        // restore.
        AuditService.log("RESTORE_MARKERLESS_PENDING_PROMOTED", data: [:])
        try keychain.storeMnemonic(pending)
        try? keychain.deletePendingMnemonic()
        restoreStateStore.setRecoveredRestorePending(true)
        return true
    }

    // MARK: - Public Forwarding (for AppState)

    /// Clears the `recovered_restore_pending` flag.
    /// Forwarded to the injected ``RestoreStateStore`` so that callers
    /// outside the lifecycle layer do not need a direct store reference.
    func clearRecoveredRestorePending() {
        restoreStateStore.clearRecoveredRestorePending()
    }
}
