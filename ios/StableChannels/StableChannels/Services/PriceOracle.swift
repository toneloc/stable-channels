import Foundation

struct PriceFeedConfig: Codable, Equatable {
    let name: String
    let urlFormat: String
    let jsonPath: [String]
}

struct NamedPrice: Equatable {
    let feedName: String
    let value: Double
}

enum PriceOracleSource: String, Equatable {
    case directUSD = "usd"
    case normalizedUSDT = "usdt"
}

struct PriceOracleResult: Equatable {
    let price: Double
    let source: PriceOracleSource
    let agreeingFeedNames: [String]
    let usdtUSD: Double?
}

enum PriceOracleFailure: Error, Equatable, CustomStringConvertible {
    case insufficientBitcoinConsensus(available: Int)
    case largeBitcoinMove(ratio: Double)
    case invalidUSDTPeg(available: Int)

    var quarantinesPrice: Bool {
        if case .largeBitcoinMove = self { return true }
        return false
    }

    var description: String {
        switch self {
        case .insufficientBitcoinConsensus(let available):
            return "BTC/USD consensus requires at least \(PriceOracle.minimumAgreeingFeeds) agreeing feeds; got \(available)"
        case .largeBitcoinMove(let ratio):
            return String(
                format: "BTC/USD median moved %.2f%% from the last trusted price, above the %.2f%% limit",
                ratio * 100,
                PriceOracle.maximumMedianMoveRatio * 100
            )
        case .invalidUSDTPeg(let available):
            return "USDT fallback requires at least \(PriceOracle.minimumAgreeingPegFeeds) agreeing peg feeds; got \(available)"
        }
    }
}

/// Shared pricing policy used by the foreground app and notification extension.
/// Direct USD books define the index. USDT books are queried only as a fallback and
/// are converted through a separately validated USDT/USD rate.
enum PriceOracle {
    static let minimumBitcoinUSD = 1_000.0
    static let maximumBitcoinUSD = 10_000_000.0
    static let minimumAgreeingFeeds = 3
    static let maximumFeedDeviationRatio = 0.05
    static let maximumMedianMoveRatio = 0.10
    static let maximumTrustedPriceAge: TimeInterval = 60

    static let minimumAgreeingPegFeeds = 3
    static let maximumUSDTPegDeviationFromDollar = 0.005
    static let maximumPegFeedDeviationRatio = 0.0025

    static let directUSDFeeds: [PriceFeedConfig] = [
        PriceFeedConfig(
            name: "Bitstamp",
            urlFormat: "https://www.bitstamp.net/api/v2/ticker/btcusd/",
            jsonPath: ["last"]
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
            name: "Gemini",
            urlFormat: "https://api.gemini.com/v1/pubticker/btcusd",
            jsonPath: ["last"]
        ),
        PriceFeedConfig(
            name: "Bullish",
            urlFormat: "https://api.exchange.bullish.com/trading-api/v1/markets/BTCUSD/tick",
            jsonPath: ["last"]
        )
    ]

    static let bitcoinUSDTFeeds: [PriceFeedConfig] = [
        PriceFeedConfig(
            name: "Binance BTC/USDT",
            urlFormat: "https://api.binance.com/api/v3/ticker/price?symbol=BTCUSDT",
            jsonPath: ["price"]
        ),
        PriceFeedConfig(
            name: "Bybit BTC/USDT",
            urlFormat: "https://api.bybit.com/v5/market/tickers?category=spot&symbol=BTCUSDT",
            jsonPath: ["result", "list", "0", "lastPrice"]
        ),
        PriceFeedConfig(
            name: "Huobi BTC/USDT",
            urlFormat: "https://api.huobi.pro/market/detail/merged?symbol=btcusdt",
            jsonPath: ["tick", "close"]
        ),
        PriceFeedConfig(
            name: "KuCoin BTC/USDT",
            urlFormat: "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=BTC-USDT",
            jsonPath: ["data", "price"]
        ),
        PriceFeedConfig(
            name: "Gate.io BTC/USDT",
            urlFormat: "https://api.gateio.ws/api/v4/spot/tickers?currency_pair=BTC_USDT",
            jsonPath: ["0", "last"]
        ),
        PriceFeedConfig(
            name: "MEXC BTC/USDT",
            urlFormat: "https://api.mexc.com/api/v3/ticker/price?symbol=BTCUSDT",
            jsonPath: ["price"]
        ),
        PriceFeedConfig(
            name: "Luno BTC/USDT",
            urlFormat: "https://api.luno.com/api/1/ticker?pair=XBTUSDT",
            jsonPath: ["last_trade"]
        ),
        PriceFeedConfig(
            name: "CoinDCX BTC/USDT",
            urlFormat: "https://public.coindcx.com/market_data/trade_history?pair=B-BTC_USDT&limit=1",
            jsonPath: ["0", "p"]
        ),
        PriceFeedConfig(
            name: "BTCTurk BTC/USDT",
            urlFormat: "https://api.btcturk.com/api/v2/ticker?pairSymbol=BTCUSDT",
            jsonPath: ["data", "0", "last"]
        )
    ]

    static let usdtUSDFeeds: [PriceFeedConfig] = [
        PriceFeedConfig(
            name: "Coinbase USDT/USD",
            urlFormat: "https://api.coinbase.com/v2/prices/USDT-USD/spot",
            jsonPath: ["data", "amount"]
        ),
        PriceFeedConfig(
            name: "Kraken USDT/USD",
            urlFormat: "https://api.kraken.com/0/public/Ticker?pair=USDTUSD",
            jsonPath: ["result", "USDTZUSD", "c"]
        ),
        PriceFeedConfig(
            name: "Bitstamp USDT/USD",
            urlFormat: "https://www.bitstamp.net/api/v2/ticker/usdtusd/",
            jsonPath: ["last"]
        ),
        PriceFeedConfig(
            name: "Bitfinex USDT/USD",
            urlFormat: "https://api-pub.bitfinex.com/v2/ticker/tUSTUSD",
            jsonPath: ["6"]
        ),
        PriceFeedConfig(
            name: "CoinGecko USDT/USD",
            urlFormat: "https://api.coingecko.com/api/v3/simple/price?ids=tether&vs_currencies=usd",
            jsonPath: ["tether", "usd"]
        )
    ]

    static func resolve(
        usdPrices: [NamedPrice],
        usdtPrices: [NamedPrice],
        pegPrices: [NamedPrice],
        lastTrustedPrice: Double?
    ) throws -> PriceOracleResult {
        do {
            let consensus = try validateBitcoinConsensus(usdPrices, lastTrustedPrice: lastTrustedPrice)
            return PriceOracleResult(
                price: consensus.price,
                source: .directUSD,
                agreeingFeedNames: consensus.feeds.map(\.feedName),
                usdtUSD: nil
            )
        } catch let failure as PriceOracleFailure where !failure.quarantinesPrice {
            let peg = try validateUSDTPeg(pegPrices)
            let normalized = usdtPrices.map {
                NamedPrice(feedName: $0.feedName, value: $0.value * peg.price)
            }
            let consensus = try validateBitcoinConsensus(normalized, lastTrustedPrice: lastTrustedPrice)
            return PriceOracleResult(
                price: consensus.price,
                source: .normalizedUSDT,
                agreeingFeedNames: consensus.feeds.map(\.feedName),
                usdtUSD: peg.price
            )
        }
    }

    static func validateBitcoinConsensus(
        _ prices: [NamedPrice],
        lastTrustedPrice: Double?
    ) throws -> (price: Double, feeds: [NamedPrice]) {
        let plausible = prices.filter { isPlausibleBitcoinPrice($0.value) }
        guard let initialMedian = median(plausible.map(\.value)) else {
            throw PriceOracleFailure.insufficientBitcoinConsensus(available: 0)
        }
        let agreeing = plausible.filter {
            relativeDeviation($0.value, initialMedian) <= maximumFeedDeviationRatio
        }
        guard agreeing.count >= minimumAgreeingFeeds,
              let acceptedMedian = median(agreeing.map(\.value)) else {
            throw PriceOracleFailure.insufficientBitcoinConsensus(available: agreeing.count)
        }
        if let lastTrustedPrice,
           isPlausibleBitcoinPrice(lastTrustedPrice) {
            let move = relativeDeviation(acceptedMedian, lastTrustedPrice)
            if move > maximumMedianMoveRatio {
                throw PriceOracleFailure.largeBitcoinMove(ratio: move)
            }
        }
        return (acceptedMedian, agreeing)
    }

    static func validateUSDTPeg(_ prices: [NamedPrice]) throws -> (price: Double, feeds: [NamedPrice]) {
        let nearDollar = prices.filter {
            $0.value.isFinite && abs($0.value - 1.0) <= maximumUSDTPegDeviationFromDollar
        }
        guard let initialMedian = median(nearDollar.map(\.value)) else {
            throw PriceOracleFailure.invalidUSDTPeg(available: 0)
        }
        let agreeing = nearDollar.filter {
            relativeDeviation($0.value, initialMedian) <= maximumPegFeedDeviationRatio
        }
        guard agreeing.count >= minimumAgreeingPegFeeds,
              let acceptedMedian = median(agreeing.map(\.value)) else {
            throw PriceOracleFailure.invalidUSDTPeg(available: agreeing.count)
        }
        return (acceptedMedian, agreeing)
    }

    static func isPlausibleBitcoinPrice(_ price: Double) -> Bool {
        price.isFinite && (minimumBitcoinUSD ... maximumBitcoinUSD).contains(price)
    }

    static func median(_ values: [Double]) -> Double? {
        guard !values.isEmpty else { return nil }
        let sorted = values.sorted()
        let midpoint = sorted.count / 2
        if sorted.count.isMultiple(of: 2) {
            return sorted[midpoint - 1] + (sorted[midpoint] - sorted[midpoint - 1]) / 2
        }
        return sorted[midpoint]
    }

    private static func relativeDeviation(_ value: Double, _ reference: Double) -> Double {
        guard value.isFinite, reference.isFinite, reference > 0 else { return .infinity }
        return abs(value - reference) / reference
    }
}
