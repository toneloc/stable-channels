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
        guard let data = mnemonic.data(using: .utf8) else {
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
        return SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess
    }
}
