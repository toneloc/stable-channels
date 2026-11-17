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
