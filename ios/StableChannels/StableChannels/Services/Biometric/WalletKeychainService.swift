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
final class WalletKeychainService {
    static let shared = WalletKeychainService()

    static var onLog: ((String, [String: Any]) -> Void)?

    private let service = "com.stablechannels.wallet"
    private let account = "seed_phrase"
    private let accessGroup = "group.com.stablechannels.app"

    private func logError(_ event: String, data: [String: Any]) {
        NSLog("[WalletKeychain] ERROR: \(event) - \(data)")
        Self.onLog?(event, data)
    }

    func storeMnemonic(_ mnemonic: String) throws {
        let trimmed = mnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw WalletKeychainError.dataConversionFailed
        }
        guard let data = trimmed.data(using: .utf8) else {
            throw WalletKeychainError.dataConversionFailed
        }

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: accessGroup
        ]

        // Delete any existing item to prevent duplicate error
        SecItemDelete(query as CFDictionary)

        let attributes: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: accessGroup,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]

        let status = SecItemAdd(attributes as CFDictionary, nil)
        guard status == errSecSuccess else {
            logError("KEYCHAIN_STORE_FAILED", data: ["status": String(status)])
            throw WalletKeychainError.accessDenied(status)
        }

        // Verify write by loading back (Issue 8)
        let loaded = try loadMnemonic()
        guard loaded == trimmed else {
            logError("KEYCHAIN_VERIFICATION_FAILED", data: [:])
            throw WalletKeychainError.dataConversionFailed
        }
    }

    func loadMnemonic() throws -> String {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: accessGroup,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)

        guard status == errSecSuccess, let data = result as? Data else {
            if status == errSecItemNotFound {
                throw WalletKeychainError.keyNotFound
            }
            logError("KEYCHAIN_LOAD_FAILED", data: ["status": String(status)])
            throw WalletKeychainError.accessDenied(status)
        }

        guard let mnemonic = String(data: data, encoding: .utf8) else {
            throw WalletKeychainError.dataConversionFailed
        }

        return mnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func deleteMnemonic() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: accessGroup
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            logError("KEYCHAIN_DELETE_FAILED", data: ["status": String(status)])
        }
    }

    func hasMnemonic() -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: accessGroup,
            kSecReturnData as String: false
        ]
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status != errSecSuccess && status != errSecItemNotFound {
            logError("KEYCHAIN_EXISTS_CHECK_FAILED", data: ["status": String(status)])
        }
        return status == errSecSuccess
    }
}
