import Foundation

enum Constants {
    // MARK: - Network

    static let satsInBTC: UInt64 = 100_000_000
    static let stableChannelTLVType: UInt64 = 13_377_331
    static let tradeMessageType = "TRADE_V1"
    static let syncMessageType = "SYNC_V1"

    // MARK: - Default Configuration

    // Overridable via TestOverrides (debug builds only) for E2E regtest runs.
    static var defaultNetwork: String { TestOverrides.shared.network ?? "bitcoin" }
    static let defaultUserAlias = "user"
    static let defaultUserPort: UInt16 = 9736
    static let defaultLSPAlias = "lsp"
    static let defaultLSPPort: UInt16 = 9735

    static var primaryChainURL: String {
        TestOverrides.shared.primaryChainUrl ?? "https://blockstream.info/api"
    }

    static var fallbackChainURL: String {
        TestOverrides.shared.fallbackChainUrl ?? "https://mempool.space/api"
    }

    static var esploraChainURLs: [String] { [primaryChainURL, fallbackChainURL] }
    static let txExplorerURL = "https://mempool.space/tx"

    static var feeRateBlockstreamURL: String { primaryChainURL }
    static let feeRateMempoolURL = "https://mempool.space"

    // MARK: - Service Endpoints

    static var lspPushRegisterURL: String {
        TestOverrides.shared.pushRegisterUrl ?? "https://stablechannels.com/api/register-push"
    }

    static var lspChannelExistsURL: String {
        TestOverrides.shared.channelExistsUrl ?? "https://stablechannels.com/api/channel-exists"
    }

    static let privacyPolicyURL = "https://stablechannels.com/privacy.html"

    static func txExplorerLink(for txid: String) -> URL? {
        URL(string: "\(txExplorerURL)/\(txid)")
    }

    static var defaultLSPPubkey: String {
        TestOverrides.shared.lspPubkey ?? "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    }

    static var defaultLSPAddress: String {
        TestOverrides.shared.lspAddress ?? "stablechannels.com:9735"
    }

    // MARK: - Timing

    static let priceCacheRefreshSecs: UInt64 = 15
    static let priceFetchTimeoutSecs: TimeInterval = 3
    /// Longer budget for the ~30-day hourly OHLC chart backfill, which is a much larger download
    /// than a single-price ticker and must not share the short per-feed ticker timeout.
    static let chartFetchTimeoutSecs: TimeInterval = 30

    // E2E override shortens both (regtest blocks are on demand).
    static var onchainWalletSyncIntervalSecs: UInt64 {
        TestOverrides.shared.syncIntervalSecs ?? 120
    }

    static var lightningWalletSyncIntervalSecs: UInt64 {
        TestOverrides.shared.syncIntervalSecs ?? 60
    }

    /// Production checks splice confirmation conservatively; regtest reuses
    /// its shorter sync cadence so mined demo transactions clear promptly.
    static var spliceConfirmationPollIntervalSecs: UInt64 {
        TestOverrides.shared.syncIntervalSecs ?? 30
    }

    static let feeRateCacheUpdateIntervalSecs: UInt64 = 1200

    static let invoiceExpirySecs: UInt32 = 3600
    static let balanceUpdateIntervalSecs: UInt64 = 30
    /// The E2E override also drives deposit detection, which shares the
    /// stability timer. Production retains the conservative 60-second tick.
    static var stabilityCheckIntervalSecs: UInt64 {
        TestOverrides.shared.syncIntervalSecs ?? 60
    }

    // MARK: - Business Logic

    static let maxRiskLevel: Int32 = 100
    static let stabilityThresholdPercent: Double = 0.1
    static let stabilityThresholdUSD: Double = 0.25
    static let stabilityPaymentCooldownSecs: UInt64 = 120
    /// A stability payment may only be sent when the Lightning wallet synced to chain within
    /// this window (two 60s background sync intervals, so one missed tick is tolerated).
    /// Keep in sync with STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS in src/constants.rs and the
    /// NSE's copy in NotificationService.swift.
    static let stabilityMaxLightningSyncAgeSecs: UInt64 = 120
    static let minDisplayUSD: Double = 2.0
    static let maxChannelUSD: Double = 100.0
    /// Stable-channel trade fee paid to the LSP as the TRADE_V1 keysend amount.
    static let stableChannelTradeFeeRate: Double = 0.01
    static let lightningDefaultForwardingFeeBaseMsat: UInt32 = 1_000
    static let lightningDefaultForwardingFeeProportionalMillionths: UInt32 = 0
    static let estimatedOnchainSendVBytes: UInt64 = 140
    static let estimatedOnchainSendAllVBytes: UInt64 = 250
    static let estimatedChannelCloseVBytes: UInt64 = 180

    // MARK: - Channel

    static let defaultChannelLifetime: UInt32 = 2016
    static let defaultMaxClientToSelfDelay: UInt32 = 1024
    static let minPaymentSizeMsat: UInt64 = 0
    static let maxPaymentSizeMsat: UInt64 = 100_000_000_000
    static let channelOverProvisioningPPM: UInt32 = 1_000_000
    static let channelOpeningFeePPM: UInt32 = 0
    static let minChannelOpeningFeeMsat: UInt64 = 0
    static let minChannelLifetime: UInt32 = 100
    static let maxProportionalLSPFeeLimitPPMMsat: UInt64 = 10_000_000

    // MARK: - Price Feeds

    /// E2E override hook: when TestOverrides supplies a local feed base, the direct-USD
    /// feeds route there; production uses the PriceOracle feed set from main.
    static var defaultPriceFeeds: [PriceFeedConfig] {
        if let base = TestOverrides.shared.priceFeedBase {
            return TestOverrides.priceFeeds(base: base)
        }
        return PriceOracle.directUSDFeeds
    }

    /// The USDT fallback must be silenced under the E2E override: real exchange prices
    /// diverge arbitrarily from the harness's mocked price, so one transient direct-feed
    /// miss would resolve real-world quotes against the mocked lastTrustedPrice, trip the
    /// largeBitcoinMove guard, and quarantine the price — blocking every send mid-suite.
    /// An empty fallback fails as insufficientBitcoinConsensus (non-quarantining) and the
    /// mocked price stays trusted until the next direct refresh.
    static var fallbackUSDTPriceFeeds: [PriceFeedConfig] {
        TestOverrides.shared.priceFeedBase != nil ? [] : PriceOracle.bitcoinUSDTFeeds
    }

    static var usdtUSDPriceFeeds: [PriceFeedConfig] {
        TestOverrides.shared.priceFeedBase != nil ? [] : PriceOracle.usdtUSDFeeds
    }

    // MARK: - RGS (Rapid Gossip Sync) Servers

    enum RGSServer {
        static let bitcoin = "https://rapidsync.lightningdevkit.org/snapshot/"
        static let signet = "https://rgs.mutinynet.com/snapshot/"
        static let testnet = "https://rapidsync.lightningdevkit.org/testnet/snapshot/"
    }

    // MARK: - Push Notifications

    static let appGroupIdentifier = "group.com.stablechannels.app"

    // MARK: - Data Directory

    static var userDataDir: URL {
        if let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) {
            return shared.appendingPathComponent("StableChannels")
                .appendingPathComponent(defaultUserAlias)
        }
        // Fallback to Application Support if App Group is unavailable
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return appSupport.appendingPathComponent("StableChannels").appendingPathComponent(defaultUserAlias)
    }
}

// MARK: - Seed Constants

enum SeedConstants {
    static let wordCount12 = 12
    static let wordCount24 = 24
    static let maxWordCount = 24
    static let clipboardClearSeconds: TimeInterval = 60
    static let defaultWordCount = 12
    static let animationDuration: TimeInterval = 0.3
    static let successDisplaySeconds: UInt64 = 1_500_000_000
}

extension Constants {
    /// URLs for the logs the app already writes today (audit log and LDK node log),
    /// filtered to those that currently exist on disk. Used by the Logs & Diagnostics export UI.
    static func exportableLogURLs() -> [URL] {
        let fileManager = FileManager.default
        let candidates = [
            userDataDir.appendingPathComponent("audit_log.txt"),
            userDataDir.appendingPathComponent("ldk-node.log")
        ]
        return candidates.filter { fileManager.fileExists(atPath: $0.path) }
    }
}
