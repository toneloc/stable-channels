import Foundation
import Security

enum KeychainError: Error, LocalizedError {
    case accessDenied
    case keyNotFound
    case keychainUnavailable

    var errorDescription: String? {
        switch self {
        case .accessDenied: return "Access denied"
        case .keyNotFound: return "Key not found"
        case .keychainUnavailable: return "Keychain unavailable"
        }
    }
}

@MainActor
@Observable
final class KeychainService {
    static let shared = KeychainService()

    private let service = "com.stablechannels.backup"
    private let account = "encryptionKey"

    // MARK: - CRUD

    func generateAndStoreKey() throws {
        if hasKey() { return }

        let keyData = try generateRandomBytes(count: 32)
        try storeKey(keyData)
    }

    func loadKey() throws -> Data {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: true,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)

        guard status == errSecSuccess, let keyData = result as? Data else {
            throw KeychainError.keyNotFound
        }
        return keyData
    }

    func deleteKey() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: true
        ]
        SecItemDelete(query as CFDictionary)
    }

    func hasKey() -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: true,
            kSecReturnData as String: false
        ]
        var result: AnyObject?
        return SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess
    }

    // MARK: - Private

    private func generateRandomBytes(count: Int) throws -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        guard status == errSecSuccess else {
            throw KeychainError.keychainUnavailable
        }
        return Data(bytes)
    }

    private func storeKey(_ keyData: Data) throws {
        // Delete any existing key first to ensure clean state
        let deleteQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: true
        ]
        SecItemDelete(deleteQuery as CFDictionary)

        // Now add the new key
        let addQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: true,
            kSecValueData as String: keyData,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlocked
        ]

        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw KeychainError.keychainUnavailable
        }
    }
}
