import Foundation
import Security

enum WalletKeychainError: Error, LocalizedError, Equatable {
    case accessDenied(OSStatus)
    case keyNotFound
    case duplicateItem(OSStatus)
    case dataConversionFailed
    case unexpectedStatus(OSStatus)

    var errorDescription: String? {
        switch self {
        case .accessDenied(let status): return "Keychain access denied (status: \(status))"
        case .keyNotFound: return "Wallet seed not found in Keychain"
        case .duplicateItem(let status): return "Keychain duplicate item (status: \(status))"
        case .dataConversionFailed: return "Mnemonic string conversion failed"
        case .unexpectedStatus(let status): return "Unexpected Keychain status (status: \(status))"
        }
    }

    static func from(status: OSStatus) -> WalletKeychainError {
        switch status {
        case errSecItemNotFound:
            return .keyNotFound
        case errSecAuthFailed, errSecInteractionNotAllowed, errSecMissingEntitlement:
            return .accessDenied(status)
        case errSecDuplicateItem:
            return .duplicateItem(status)
        default:
            return .unexpectedStatus(status)
        }
    }
}

protocol MnemonicStorageProtocol {
    func storeMnemonic(_ mnemonic: String) throws
    func loadMnemonic() throws -> String
    func deleteMnemonic() throws
    func hasMnemonic() throws -> Bool

    func storePendingMnemonic(_ mnemonic: String) throws
    func loadPendingMnemonic() throws -> String
    func deletePendingMnemonic() throws
    func hasPendingMnemonic() throws -> Bool
}

/// Keychain-backed secure storage service for the wallet mnemonic.
///
/// ...
class WalletKeychainService: MnemonicStorageProtocol {
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

    func hasMnemonic() throws -> Bool {
        return try hasMnemonicInternal(accountName: account)
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

    func hasPendingMnemonic() throws -> Bool {
        return try hasMnemonicInternal(accountName: pendingAccount)
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

        let exists = (try? hasMnemonicInternal(accountName: accountName)) ?? false
        if exists {
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
                throw WalletKeychainError.from(status: status)
            }
        } else {
            var attributes = base
            attributes[kSecValueData as String] = data
            attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let status = SecItemAdd(attributes as CFDictionary, nil)
            guard status == errSecSuccess else {
                logError("KEYCHAIN_STORE_FAILED", data: ["op": "add", "account": accountName, "status": String(status)])
                throw WalletKeychainError.from(status: status)
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
            throw WalletKeychainError.from(status: status)
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
            throw WalletKeychainError.from(status: status)
        }
    }

    private func hasMnemonicInternal(accountName: String) throws -> Bool {
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
        if status == errSecSuccess {
            return true
        } else if status == errSecItemNotFound {
            return false
        } else {
            logError("KEYCHAIN_EXISTS_CHECK_FAILED", data: ["account": accountName, "status": String(status)])
            throw WalletKeychainError.from(status: status)
        }
    }
}
