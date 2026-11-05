import Foundation

// Compiled into both the app and the notification extension, so both nodes sync with the same limits.
extension Constants {
    static let onchainWalletSyncTimeoutSecs: UInt64 = 60
    static let lightningWalletSyncTimeoutSecs: UInt64 = 60
    static let txBroadcastTimeoutSecs: UInt64 = 30
    static let esploraRequestTimeoutSecs: UInt8 = 15
}
