import Foundation
import Security

enum WalletKeychainError: Error, LocalizedError {
    case accessDenied(OSStatus)
    case keyNotFound
    case dataConversionFailed

    var errorDescription: String? {
        switch self {
        case .accessDenied(let status): return "Keychain access denied (status: \(status))"
        case .keyNotFound: return "Wallet seed not found in Keychain"
        case .dataConversionFailed: return "Mnemonic string conversion failed"
        }
    }
}

/// Keychain-backed secure storage service for the wallet mnemonic.
///
/// ### Security Architecture Rationale
/// This service uses `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` without explicit biometric access control
/// (`kSecAccessControl`).
///
/// **Why no direct biometric/Face ID gating on the Keychain item:**
/// - The background extension (`NotificationService`) runs in the background and must boot the LDK Node to process
/// incoming payments
///   without requiring user interaction.
///   Applying `.biometryCurrentSet` or custom access control would prompt the user for Face ID on every background
/// receive execution,
///   which would cause background processing to hang or fail silently when the device is locked in a pocket/bag.
///
/// **Why this remains secure:**
/// 1. **Sandbox Isolation:** iOS sandboxing ensures no other application on the device can access this app's storage or
/// memory space.
/// 2. **Shared App Group Domain:** The Keychain item is scoped tightly to the `group.com.stablechannels.app` access
/// group. Only the main app
///    and our official Notification Service Extension are part of this group.
/// 3. **Non-Syncable Storage:** The `ThisDeviceOnly` attribute guarantees the item is never synced to iCloud or
/// transferred to other devices via backups.
/// 4. **Feature-Layer Gating:** Biometric authentication (via `BiometricToggleCoordinator`) is enforced at the
/// UI/feature layer for high-privilege
///    actions (such as unlocking the application or manually viewing the seed phrase).
class WalletKeychainService {
    static let shared = WalletKeychainService()

    static var onLog: ((String, [String: Any]) -> Void)?

    private let service: String
    private let account: String
    private let pendingAccount: String
    private let accessGroup: String?

    init(
        service: String = "com.stablechannels.wallet",
        account: String = "seed_phrase",
        accessGroup: String? = "group.com.stablechannels.app"
    ) {
        self.service = service
        self.account = account
        self.pendingAccount = account + "_pending"
        self.accessGroup = accessGroup
    }

    private func logError(_ event: String, data: [String: Any]) {
        NSLog("[WalletKeychain] ERROR: \(event) - \(data)")
        Self.onLog?(event, data)
    }

    func storeMnemonic(_ mnemonic: String) throws {
        try storeMnemonicInternal(mnemonic, accountName: account)
    }

    func loadMnemonic() throws -> String {
        return try loadMnemonicInternal(accountName: account)
    }

    func deleteMnemonic() throws {
        try deleteMnemonicInternal(accountName: account)
    }

    func hasMnemonic() -> Bool {
        return hasMnemonicInternal(accountName: account)
    }

    // MARK: - Pending Mnemonic (Staged Restore Transaction)

    func storePendingMnemonic(_ mnemonic: String) throws {
        try storeMnemonicInternal(mnemonic, accountName: pendingAccount)
    }

    func loadPendingMnemonic() throws -> String {
        return try loadMnemonicInternal(accountName: pendingAccount)
    }

    func deletePendingMnemonic() throws {
        try deleteMnemonicInternal(accountName: pendingAccount)
    }

    func hasPendingMnemonic() -> Bool {
        return hasMnemonicInternal(accountName: pendingAccount)
    }

    // MARK: - Generic Internal Implementations

    private func storeMnemonicInternal(_ mnemonic: String, accountName: String) throws {
        let trimmed = mnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw WalletKeychainError.dataConversionFailed
        }
        guard let data = trimmed.data(using: .utf8) else {
            throw WalletKeychainError.dataConversionFailed
        }

        // No-op if the stored value already matches — avoids any unnecessary write.
        if let existing = try? loadMnemonicInternal(accountName: accountName), existing == trimmed {
            return
        }

        var base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountName
        ]
        if let group = accessGroup, !group.isEmpty {
            base[kSecAttrAccessGroup as String] = group
        }

        if hasMnemonicInternal(accountName: accountName) {
            let attributesToUpdate: [String: Any] = [
                kSecValueData as String: data,
                kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            ]
            let status = SecItemUpdate(base as CFDictionary, attributesToUpdate as CFDictionary)
            guard status == errSecSuccess else {
                logError(
                    "KEYCHAIN_STORE_FAILED",
                    data: ["op": "update", "account": accountName, "status": String(status)]
                )
                throw WalletKeychainError.accessDenied(status)
            }
        } else {
            var attributes = base
            attributes[kSecValueData as String] = data
            attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let status = SecItemAdd(attributes as CFDictionary, nil)
            guard status == errSecSuccess else {
                logError("KEYCHAIN_STORE_FAILED", data: ["op": "add", "account": accountName, "status": String(status)])
                throw WalletKeychainError.accessDenied(status)
            }
        }

        // Verify write by loading back
        let loaded = try loadMnemonicInternal(accountName: accountName)
        guard loaded == trimmed else {
            logError("KEYCHAIN_VERIFICATION_FAILED", data: ["account": accountName])
            throw WalletKeychainError.dataConversionFailed
        }
    }

    private func loadMnemonicInternal(accountName: String) throws -> String {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountName,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        if let group = accessGroup, !group.isEmpty {
            query[kSecAttrAccessGroup as String] = group
        }

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)

        guard status == errSecSuccess, let data = result as? Data else {
            if status == errSecItemNotFound {
                throw WalletKeychainError.keyNotFound
            }
            logError("KEYCHAIN_LOAD_FAILED", data: ["account": accountName, "status": String(status)])
            throw WalletKeychainError.accessDenied(status)
        }

        guard let mnemonic = String(data: data, encoding: .utf8) else {
            throw WalletKeychainError.dataConversionFailed
        }

        return mnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func deleteMnemonicInternal(accountName: String) throws {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountName
        ]
        if let group = accessGroup, !group.isEmpty {
            query[kSecAttrAccessGroup as String] = group
        }
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            logError("KEYCHAIN_DELETE_FAILED", data: ["account": accountName, "status": String(status)])
            throw WalletKeychainError.accessDenied(status)
        }
    }

    private func hasMnemonicInternal(accountName: String) -> Bool {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountName,
            kSecReturnData as String: false
        ]
        if let group = accessGroup, !group.isEmpty {
            query[kSecAttrAccessGroup as String] = group
        }
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status != errSecSuccess && status != errSecItemNotFound {
            logError("KEYCHAIN_EXISTS_CHECK_FAILED", data: ["account": accountName, "status": String(status)])
        }
        return status == errSecSuccess
    }
}
