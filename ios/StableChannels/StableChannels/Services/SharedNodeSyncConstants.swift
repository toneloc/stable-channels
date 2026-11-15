import Foundation
import LDKNode

// Compiled into both the app and the notification extension, so both nodes sync with the same limits.
extension Constants {
    static let onchainWalletSyncTimeoutSecs: UInt64 = 60
    static let lightningWalletSyncTimeoutSecs: UInt64 = 60
    static let txBroadcastTimeoutSecs: UInt64 = 30
    static let esploraRequestTimeoutSecs: UInt8 = 15

    /// The node's Esplora sync config; the intervals and fee-rate timeout come from each target's own `Constants`.
    static var esploraSyncConfig: EsploraSyncConfig {
        EsploraSyncConfig(
            backgroundSyncConfig: BackgroundSyncConfig(
                onchainWalletSyncIntervalSecs: onchainWalletSyncIntervalSecs,
                lightningWalletSyncIntervalSecs: lightningWalletSyncIntervalSecs,
                feeRateCacheUpdateIntervalSecs: feeRateCacheUpdateIntervalSecs
            ),
            timeoutsConfig: SyncTimeoutsConfig(
                onchainWalletSyncTimeoutSecs: onchainWalletSyncTimeoutSecs,
                lightningWalletSyncTimeoutSecs: lightningWalletSyncTimeoutSecs,
                feeRateCacheUpdateTimeoutSecs: feeRateCacheUpdateTimeoutSecs,
                txBroadcastTimeoutSecs: txBroadcastTimeoutSecs,
                perRequestTimeoutSecs: esploraRequestTimeoutSecs
            )
        )
    }
}

extension Error {
    /// Determines whether a startup error is an Esplora feerate estimation failure or timeout
    /// eligible for provider failover. Non-Esplora errors (database, storage, lock, entropy)
    /// return false and should not be retried.
    var isRetryableEsploraStartupError: Bool {
        // Typed match only: NodeError's description is a human sentence ("Failed to
        // update fee rate estimates."), so string-matching case names never fires for
        // real errors and could false-positive on a bridged error carrying the text.
        guard let nodeError = self as? NodeError else { return false }
        switch nodeError {
        case .FeerateEstimationUpdateFailed, .FeerateEstimationUpdateTimeout:
            return true
        default:
            return false
        }
    }
}
