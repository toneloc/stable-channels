import Foundation

enum Constants {

    // MARK: - Network

    static let satsInBTC: UInt64 = 100_000_000
    static let stableChannelTLVType: UInt64 = 13_377_331
    static let tradeMessageType = "TRADE_V1"
    static let syncMessageType = "SYNC_V1"

    // MARK: - Default Configuration

    static let defaultNetwork = "bitcoin"
    static let defaultUserAlias = "user"
    static let defaultUserPort: UInt16 = 9736
    static let defaultLSPAlias = "lsp"
    static let defaultLSPPort: UInt16 = 9735

    static let primaryChainURL = "https://blockstream.info/api"
    static let fallbackChainURL = "https://mempool.space/api"
    static let defaultLSPPubkey = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    static let defaultLSPAddress = "34.198.44.89:9735"
    static let defaultGatewayPubkey = "03da1c27ca77872ac5b3e568af30673e599a47a5e4497f85c7b5da42048807b3ed"
    static let defaultGatewayAddress = "213.174.156.80:9735"

    // MARK: - Timing

    static let priceCacheRefreshSecs: UInt64 = 5
    static let priceFetchRetryDelayMs: UInt64 = 300
    static let priceFetchMaxRetries = 3

    static let onchainWalletSyncIntervalSecs: UInt64 = 120
    static let lightningWalletSyncIntervalSecs: UInt64 = 60
    static let feeRateCacheUpdateIntervalSecs: UInt64 = 1200

    static let invoiceExpirySecs: UInt32 = 3600
    static let balanceUpdateIntervalSecs: UInt64 = 30
    static let stabilityCheckIntervalSecs: UInt64 = 60

    // MARK: - Business Logic

    static let maxRiskLevel: Int32 = 100
    static let stabilityThresholdPercent: Double = 0.1
    static let stabilityThresholdUSD: Double = 0.10
    static let stabilityPaymentCooldownSecs: UInt64 = 120
    static let minDisplayUSD: Double = 2.0
    static let maxChannelUSD: Double = 100.0


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

    static let defaultPriceFeeds: [PriceFeedConfig] = [
        PriceFeedConfig(
            name: "Bitstamp",
            urlFormat: "https://www.bitstamp.net/api/v2/ticker/btcusd/",
            jsonPath: ["last"]
        ),
        PriceFeedConfig(
            name: "CoinGecko",
            urlFormat: "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
            jsonPath: ["bitcoin", "usd"]
        ),
        PriceFeedConfig(
            name: "Kraken",
            urlFormat: "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD",
            jsonPath: ["result", "XXBTZUSD", "c"]
        ),
        PriceFeedConfig(
            name: "Coinbase",
            urlFormat: "https://api.coinbase.com/v2/prices/spot?currency=USD",
            jsonPath: ["data", "amount"]
        ),
        PriceFeedConfig(
            name: "Blockchain.com",
            urlFormat: "https://blockchain.info/ticker",
            jsonPath: ["USD", "last"]
        ),
    ]

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

struct PriceFeedConfig: Codable {
    let name: String
    let urlFormat: String
    let jsonPath: [String]
}
