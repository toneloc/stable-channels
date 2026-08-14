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
    static let esploraChainURLs: [String] = [primaryChainURL, fallbackChainURL]
    static let txExplorerURL = "https://mempool.space/tx"

    static let feeRateBlockstreamURL = primaryChainURL
    static let feeRateMempoolURL = "https://mempool.space"

    // MARK: - Service Endpoints

    static let lspPushRegisterURL = "https://stablechannels.com/api/register-push"
    static let lspChannelExistsURL = "https://stablechannels.com/api/channel-exists"
    static let privacyPolicyURL = "https://stablechannels.com/privacy.html"

    static func txExplorerLink(for txid: String) -> URL? {
        URL(string: "\(txExplorerURL)/\(txid)")
    }

    static let defaultLSPPubkey = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    static let defaultLSPAddress = "stablechannels.com:9735"

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
    static let stabilityThresholdUSD: Double = 0.25
    static let stabilityPaymentCooldownSecs: UInt64 = 120
    static let minDisplayUSD: Double = 2.0
    static let maxChannelUSD: Double = 100.0
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

    static let defaultPriceFeeds: [PriceFeedConfig] = [
        // Tier 1 — top volume global exchanges
        PriceFeedConfig(
            name: "Binance",
            urlFormat: "https://api.binance.com/api/v3/ticker/24hr?symbol=BTCUSDT",
            jsonPath: ["lastPrice"]
        ),
        PriceFeedConfig(
            name: "Kraken",
            urlFormat: "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD",
            jsonPath: ["result", "XXBTZUSD", "c"]
        ),
        PriceFeedConfig(
            name: "Coinbase",
            urlFormat: "https://api.coinbase.com/v2/prices/BTC-USD/spot",
            jsonPath: ["data", "amount"]
        ),
        PriceFeedConfig(
            name: "Bitfinex",
            urlFormat: "https://api-pub.bitfinex.com/v2/ticker/tBTCUSD",
            jsonPath: ["6"]
        ),
        PriceFeedConfig(
            name: "CoinGecko",
            urlFormat: "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
            jsonPath: ["bitcoin", "usd"]
        ),
        PriceFeedConfig(
            name: "Blockchain.com",
            urlFormat: "https://blockchain.info/ticker",
            jsonPath: ["USD", "last"]
        ),
        PriceFeedConfig(
            name: "Coinlore",
            urlFormat: "https://api.coinlore.net/api/ticker/?id=90",
            jsonPath: ["0", "price_usd"]
        ),
        PriceFeedConfig(
            name: "Bybit",
            urlFormat: "https://api.bybit.com/v5/market/tickers?category=spot&symbol=BTCUSDT",
            jsonPath: ["result", "list", "0", "lastPrice"]
        ),
        PriceFeedConfig(
            name: "Huobi",
            urlFormat: "https://api.huobi.pro/market/detail/merged?symbol=btcusdt",
            jsonPath: ["tick", "close"]
        ),
        // Tier 2 — regional coverage
        PriceFeedConfig(
            name: "KuCoin",
            urlFormat: "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=BTC-USDT",
            jsonPath: ["data", "price"]
        ),
        PriceFeedConfig(
            name: "Gate.io",
            urlFormat: "https://api.gateio.ws/api/v4/spot/tickers?currency_pair=BTC_USDT",
            jsonPath: ["0", "last"]
        ),
        PriceFeedConfig(
            name: "MEXC",
            urlFormat: "https://api.mexc.com/api/v3/ticker/24hr?symbol=BTCUSDT",
            jsonPath: ["lastPrice"]
        ),
        PriceFeedConfig(
            name: "Yadio",
            urlFormat: "https://api.yadio.io/exrates/BTC",
            jsonPath: ["BTC", "USD"]
        ),
        PriceFeedConfig(
            name: "Luno",
            urlFormat: "https://api.luno.com/api/1/tickers",
            jsonPath: ["last_trade"],
            filterKey: "pair",
            filterValue: "XBTUSDT"
        ),
        PriceFeedConfig(
            name: "CoinDCX",
            urlFormat: "https://api.coindcx.com/exchange/ticker",
            jsonPath: ["last_price"],
            filterKey: "market",
            filterValue: "BTCUSDT"
        ),
        PriceFeedConfig(
            name: "BTCTurk",
            urlFormat: "https://api.btcturk.com/api/v2/ticker",
            jsonPath: ["last"],
            filterKey: "pair",
            filterValue: "BTCUSDT"
        ),
        PriceFeedConfig(
            name: "Mempool.space",
            urlFormat: "https://mempool.space/api/v1/prices",
            jsonPath: ["USD"]
        )
    ]

    // MARK: - RGS (Rapid Gossip Sync) Servers

    enum RGSServer {
        static let bitcoin = "https://rapidsync.lightningdevkit.org/snapshot/"
        static let signet = "https://rgs.mutinynet.org/snapshot/"
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
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return appSupport.appendingPathComponent("StableChannels").appendingPathComponent(defaultUserAlias)
    }
}

struct PriceFeedConfig: Codable {
    let name: String
    let urlFormat: String
    let jsonPath: [String]
    let filterKey: String?
    let filterValue: String?

    init(name: String, urlFormat: String, jsonPath: [String], filterKey: String? = nil, filterValue: String? = nil) {
        self.name = name
        self.urlFormat = urlFormat
        self.jsonPath = jsonPath
        self.filterKey = filterKey
        self.filterValue = filterValue
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
    static func exportableLogURLs() -> [URL] {
        let fileManager = FileManager.default
        let candidates = [
            userDataDir.appendingPathComponent("audit_log.txt"),
            userDataDir.appendingPathComponent("ldk-node.log")
        ]
        return candidates.filter { fileManager.fileExists(atPath: $0.path) }
    }
}
