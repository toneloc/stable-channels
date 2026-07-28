import Foundation

struct LSPConfig: Codable, Equatable {
    let alias: String
    let pubkey: String
    let address: String
    let token: String?

    static let `default` = LSPConfig(
        alias: "stablechannels.com",
        pubkey: Constants.defaultLSPPubkey,
        address: Constants.defaultLSPAddress,
        token: nil
    )

    // MARK: - Persistence

    private static let userDefaultsKey = "active_lsp_config"

    static func load() -> LSPConfig {
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        guard let data = shared?.data(forKey: userDefaultsKey),
              let config = try? JSONDecoder().decode(LSPConfig.self, from: data)
        else {
            return .default
        }
        return config
    }

    func save() {
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        if let data = try? JSONEncoder().encode(self) {
            shared?.set(data, forKey: Self.userDefaultsKey)
        }
    }

    static func resetToDefault() {
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.removeObject(forKey: userDefaultsKey)
    }

    // MARK: - Validation

    static func isValidPubkey(_ key: String) -> Bool {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count == 66 else { return false }
        guard trimmed.hasPrefix("02") || trimmed.hasPrefix("03") else { return false }
        return trimmed.allSatisfy(\.isHexDigit)
    }

    static func isValidAddress(_ addr: String) -> Bool {
        let trimmed = addr.trimmingCharacters(in: .whitespacesAndNewlines)
        let parts = trimmed.split(separator: ":", maxSplits: 1)
        guard parts.count == 2,
              !parts[0].isEmpty,
              UInt16(parts[1]) != nil
        else {
            return false
        }
        return true
    }

    var isDefault: Bool {
        self == LSPConfig.default
    }
}
