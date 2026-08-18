import Foundation

enum MnemonicMigrationError: Error, LocalizedError {
    case seedMismatch

    var errorDescription: String? {
        switch self {
        case .seedMismatch:
            return "Mismatched seed storage detected. The secure Keychain seed does not match the legacy plaintext backup."
        }
    }
}

/// Responsible solely for encrypted-first mnemonic loading and legacy plaintext migration.
enum MnemonicMigrator {
    /// Normalizes internal and external whitespace for canonical BIP-39 mnemonic comparison.
    static func canonicalizeMnemonic(_ mnemonic: String) -> String {
        mnemonic
            .split(whereSeparator: \.isWhitespace)
            .joined(separator: " ")
    }

    /// Loads the stored mnemonic from Keychain, or migrates an existing legacy plaintext file to Keychain.
    ///
    /// Encrypted-first rule: If a Keychain entry exists, it is authoritative.
    @discardableResult
    static func loadOrMigrateMnemonic(
        keychain: any MnemonicStorageProtocol = WalletKeychainService.shared,
        legacyPath: URL = Constants.userDataDir.appendingPathComponent("seed_phrase"),
        logError: ((String, [String: Any]) -> Void)? = { event, data in AuditService.log(event, data: data) }
    ) throws -> String? {
        // 1. Encrypted-first: an existing Keychain seed is authoritative.
        do {
            let keychainMnemonic = try keychain.loadMnemonic()
            let canonicalKeychain = canonicalizeMnemonic(keychainMnemonic)
            // Reconcile lingering legacy plaintext file if present
            if let plaintext = try? String(contentsOfFile: legacyPath.path, encoding: .utf8) {
                let canonicalPlaintext = canonicalizeMnemonic(plaintext)
                if !canonicalPlaintext.isEmpty, canonicalPlaintext != canonicalKeychain {
                    logError?("KEYCHAIN_PLAINTEXT_MISMATCH", [:])
                    throw MnemonicMigrationError.seedMismatch
                }
                // A matching plaintext file is deliberately RETAINED as rollback insurance:
                // older builds treat "no seed files" as a brand-new wallet and wipe the
                // channel database. Plaintext deletion ships in a later release, once no
                // earlier build remains installable (staged rollout, step 1 of 2).
            }
            return canonicalKeychain
        } catch WalletKeychainError.keyNotFound {
            // Keychain is genuinely empty; plaintext migration is allowed.
        } catch let mismatch as MnemonicMigrationError {
            throw mismatch // Propagate mismatch error up to block startup
        } catch {
            logError?("KEYCHAIN_LOAD_FAILED", ["error": error.localizedDescription])
            throw error // Fail closed: operational Keychain error must halt startup!
        }

        // 2. Keychain empty — migrate plaintext if present.
        guard let words = try? String(contentsOfFile: legacyPath.path, encoding: .utf8) else { return nil }
        let canonicalWords = canonicalizeMnemonic(words)
        guard !canonicalWords.isEmpty else { return nil }

        do {
            try keychain.storeMnemonic(canonicalWords)
            // The plaintext file is deliberately RETAINED after migration (rollback
            // insurance for older builds — see the note above). Deletion is a later release.
            return canonicalWords
        } catch {
            logError?("KEYCHAIN_MIGRATION_FAILED", ["error": error.localizedDescription])
            throw error // Fail closed: migration failure must halt startup, never continue in plaintext!
        }
    }
}
