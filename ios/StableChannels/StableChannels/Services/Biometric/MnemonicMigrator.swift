import Foundation

/// Responsible solely for encrypted-first mnemonic loading and legacy plaintext migration.
enum MnemonicMigrator {
    /// Loads the stored mnemonic from Keychain, or migrates an existing legacy plaintext file to Keychain.
    ///
    /// Encrypted-first rule: If a Keychain entry exists, it is authoritative.
    @discardableResult
    static func loadOrMigrateMnemonic(
        keychain: WalletKeychainService = .shared,
        legacyPath: URL = Constants.userDataDir.appendingPathComponent("seed_phrase"),
        logError: ((String, [String: Any]) -> Void)? = { event, data in AuditService.log(event, data: data) }
    ) -> String? {
        // 1. Encrypted-first: an existing Keychain seed is authoritative.
        do {
            let keychainMnemonic = try keychain.loadMnemonic()
            // Reconcile lingering legacy plaintext file if present
            if let plaintext = try? String(contentsOfFile: legacyPath.path, encoding: .utf8) {
                let trimmed = plaintext.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty, trimmed != keychainMnemonic {
                    logError?("KEYCHAIN_PLAINTEXT_MISMATCH", [:])
                    return keychainMnemonic
                }
                do {
                    try FileManager.default.removeItem(at: legacyPath)
                } catch {
                    logError?("KEYCHAIN_PLAINTEXT_DELETE_FAILED", ["error": error.localizedDescription])
                }
            }
            return keychainMnemonic
        } catch WalletKeychainError.keyNotFound {
            // Keychain is genuinely empty; plaintext migration is allowed.
        } catch {
            logError?("KEYCHAIN_LOAD_FAILED", ["error": error.localizedDescription])
            return nil // Fail closed: leave both stores untouched
        }

        // 2. Keychain empty — migrate plaintext if present.
        guard let words = try? String(contentsOfFile: legacyPath.path, encoding: .utf8) else { return nil }
        let trimmed = words.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        do {
            try keychain.storeMnemonic(trimmed)
            do {
                try FileManager.default.removeItem(at: legacyPath)
            } catch {
                logError?("KEYCHAIN_PLAINTEXT_DELETE_FAILED", ["error": error.localizedDescription])
            }
            return trimmed
        } catch {
            logError?("KEYCHAIN_MIGRATION_FAILED", ["error": error.localizedDescription])
            return trimmed // Return candidate so app still functions even if Keychain write failed
        }
    }
}
