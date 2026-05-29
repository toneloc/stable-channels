import Foundation
import Security

@MainActor
@Observable
final class KeychainService {
    static let shared = KeychainService()

    private let service = "com.stablechannels.backup"
    private let account = "encryptionKey"

    // MARK: - CRUD

    func generateAndStoreKey() throws {
        let keyData = generateRandomBytes(count: 32)
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
            throw BackupError.keyNotFound
        }
        return keyData
    }

    func deleteKey() {
        SecItemDelete([
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account
        ] as CFDictionary)
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

    private func generateRandomBytes(count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }

    private func storeKey(_ keyData: Data) throws {
        let baseQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: true
        ]

        var result: AnyObject?
        let lookupStatus = SecItemCopyMatching(baseQuery as CFDictionary, &result)

        if lookupStatus == errSecSuccess {
            return
        }

        var addAttrs: [String: Any] = baseQuery
        addAttrs[kSecValueData as String] = keyData
        addAttrs[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlocked
        let addStatus = SecItemAdd(addAttrs as CFDictionary, nil)
        if addStatus != errSecSuccess {
            throw BackupError.keychainUnavailable
        }
    }
}
