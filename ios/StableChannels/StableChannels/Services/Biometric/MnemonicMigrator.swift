import Foundation

enum MnemonicMigrationError: Error, LocalizedError, Equatable {
    case seedMismatch
    case plaintextUnreadable

    var errorDescription: String? {
        switch self {
        case .seedMismatch:
            return "Mismatched seed storage detected. The secure Keychain seed does not match the legacy plaintext backup."
        case .plaintextUnreadable:
            return "The legacy seed backup exists but could not be read."
        }
    }
}

/// Responsible solely for encrypted-first mnemonic loading and legacy plaintext migration.
enum MnemonicMigrator {
    /// Normalizes casing and whitespace for canonical BIP-39 mnemonic comparison.
    static func canonicalizeMnemonic(_ mnemonic: String) -> String {
        mnemonic
            .lowercased()
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
                // Plaintext matches or is empty: permanently delete the plaintext seed file
                try? FileManager.default.removeItem(at: legacyPath)
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

        // 2. Keychain empty — migrate plaintext if present. A file that EXISTS but cannot
        // be read (permissions, I/O error, partial write) must not be treated as absent:
        // returning nil here is what routes NodeService.start() toward wipe-and-generate,
        // so this is the one place an unread error would destroy a wallet. Fail closed.
        let legacyExists = FileManager.default.fileExists(atPath: legacyPath.path)
        guard let words = try? String(contentsOfFile: legacyPath.path, encoding: .utf8) else {
            if legacyExists {
                logError?("PLAINTEXT_SEED_UNREADABLE", [:])
                throw MnemonicMigrationError.plaintextUnreadable
            }
            return nil
        }
        let canonicalWords = canonicalizeMnemonic(words)
        guard !canonicalWords.isEmpty else { return nil }

        do {
            try keychain.storeMnemonic(canonicalWords)
            // Permanently delete plaintext seed file after verified Keychain storage
            try? FileManager.default.removeItem(at: legacyPath)
            return canonicalWords
        } catch {
            logError?("KEYCHAIN_MIGRATION_FAILED", ["error": error.localizedDescription])
            throw error // Fail closed: migration failure must halt startup, never continue in plaintext!
        }
    }
}
