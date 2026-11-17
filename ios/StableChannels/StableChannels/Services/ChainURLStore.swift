import Foundation

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
