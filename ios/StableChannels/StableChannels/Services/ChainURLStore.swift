import Foundation

// Compiled into both the app and the notification extension, so both read and write the same keys.
extension Constants {
    /// App-group UserDefaults keys the app and the notification extension share.
    enum SharedDefaultsKey {
        /// The Esplora URL the app last started a node against.
        static let workingChainURL = "esplora_chain_url"
        /// When the app was last active; the extension leaves the node to a live app.
        static let mainAppLastActive = "main_app_last_active"
    }
}

/// Remembers the Esplora URL a node last started against, so the next start (app or
/// notification extension) begins with a known-working provider.
protocol ChainURLStore {
    func load() -> String?
    func save(_ url: String)
}

struct UserDefaultsChainURLStore: ChainURLStore {
    let defaults: UserDefaults?

    func load() -> String? {
        defaults?.string(forKey: Constants.SharedDefaultsKey.workingChainURL)
    }

    func save(_ url: String) {
        defaults?.set(url, forKey: Constants.SharedDefaultsKey.workingChainURL)
    }
}
