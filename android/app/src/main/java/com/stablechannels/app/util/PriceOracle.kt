package com.stablechannels.app.util

import kotlin.math.abs

data class NamedPrice(val feedName: String, val value: Double)

enum class PriceOracleSource {
    DIRECT_USD,
    NORMALIZED_USDT
}

data class PriceOracleResult(
    val price: Double,
    val source: PriceOracleSource,
    val agreeingFeedNames: List<String>,
    val usdtUsd: Double? = null
)

class PriceOracleException(
    message: String,
    val quarantinesPrice: Boolean = false
) : Exception(message)

/**
 * Shared Android pricing policy. Direct BTC/USD books define the index. BTC/USDT books are
 * queried only after direct USD quorum fails and are normalized through a validated USDT/USD peg.
 */
object PriceOracle {
    const val MINIMUM_BITCOIN_USD = 1_000.0
    const val MAXIMUM_BITCOIN_USD = 10_000_000.0
    const val MINIMUM_AGREEING_FEEDS = 3
    const val MAXIMUM_FEED_DEVIATION_RATIO = 0.05
    const val MAXIMUM_MEDIAN_MOVE_RATIO = 0.10
    const val MAXIMUM_TRUSTED_PRICE_AGE_SECS = 60L

    const val MINIMUM_AGREEING_PEG_FEEDS = 3
    const val MAXIMUM_USDT_PEG_DEVIATION_FROM_DOLLAR = 0.005
    const val MAXIMUM_PEG_FEED_DEVIATION_RATIO = 0.0025

    val DIRECT_USD_FEEDS = listOf(
        PriceFeedConfig("Bitstamp", "https://www.bitstamp.net/api/v2/ticker/btcusd/", listOf("last")),
        PriceFeedConfig("Kraken", "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD", listOf("result", "XXBTZUSD", "c")),
        PriceFeedConfig("Coinbase", "https://api.coinbase.com/v2/prices/BTC-USD/spot", listOf("data", "amount")),
        PriceFeedConfig("Bitfinex", "https://api-pub.bitfinex.com/v2/ticker/tBTCUSD", listOf("6")),
        PriceFeedConfig("Gemini", "https://api.gemini.com/v1/pubticker/btcusd", listOf("last")),
        PriceFeedConfig("Bullish", "https://api.exchange.bullish.com/trading-api/v1/markets/BTCUSD/tick", listOf("last"))
    )

    val BITCOIN_USDT_FEEDS = listOf(
        PriceFeedConfig("Binance BTC/USDT", "https://api.binance.com/api/v3/ticker/price?symbol=BTCUSDT", listOf("price")),
        PriceFeedConfig("Bybit BTC/USDT", "https://api.bybit.com/v5/market/tickers?category=spot&symbol=BTCUSDT", listOf("result", "list", "0", "lastPrice")),
        PriceFeedConfig("Huobi BTC/USDT", "https://api.huobi.pro/market/detail/merged?symbol=btcusdt", listOf("tick", "close")),
        PriceFeedConfig("KuCoin BTC/USDT", "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=BTC-USDT", listOf("data", "price")),
        PriceFeedConfig("Gate.io BTC/USDT", "https://api.gateio.ws/api/v4/spot/tickers?currency_pair=BTC_USDT", listOf("0", "last")),
        PriceFeedConfig("MEXC BTC/USDT", "https://api.mexc.com/api/v3/ticker/price?symbol=BTCUSDT", listOf("price")),
        PriceFeedConfig("Luno BTC/USDT", "https://api.luno.com/api/1/ticker?pair=XBTUSDT", listOf("last_trade")),
        PriceFeedConfig("CoinDCX BTC/USDT", "https://public.coindcx.com/market_data/trade_history?pair=B-BTC_USDT&limit=1", listOf("0", "p")),
        PriceFeedConfig("BTCTurk BTC/USDT", "https://api.btcturk.com/api/v2/ticker?pairSymbol=BTCUSDT", listOf("data", "0", "last"))
    )

    val USDT_USD_FEEDS = listOf(
        PriceFeedConfig("Coinbase USDT/USD", "https://api.coinbase.com/v2/prices/USDT-USD/spot", listOf("data", "amount")),
        PriceFeedConfig("Kraken USDT/USD", "https://api.kraken.com/0/public/Ticker?pair=USDTUSD", listOf("result", "USDTZUSD", "c")),
        PriceFeedConfig("Bitstamp USDT/USD", "https://www.bitstamp.net/api/v2/ticker/usdtusd/", listOf("last")),
        PriceFeedConfig("Bitfinex USDT/USD", "https://api-pub.bitfinex.com/v2/ticker/tUSTUSD", listOf("6")),
        PriceFeedConfig("CoinGecko USDT/USD", "https://api.coingecko.com/api/v3/simple/price?ids=tether&vs_currencies=usd", listOf("tether", "usd")),
        // Disjoint-host peg sources: the four exchange peg feeds above share hosts with the
        // direct-USD tier, so without these the fallback's peg gate would fail exactly when
        // the primary tier is unreachable — the outage the fallback exists to survive.
        PriceFeedConfig("Crypto.com USDT/USD", "https://api.crypto.com/exchange/v1/public/get-tickers?instrument_name=USDT_USD", listOf("result", "data", "0", "a")),
        PriceFeedConfig("OKX USDT/USD", "https://www.okx.com/api/v5/market/ticker?instId=USDT-USD", listOf("data", "0", "last"))
    )

    fun resolve(
        usdPrices: List<NamedPrice>,
        usdtPrices: List<NamedPrice>,
        pegPrices: List<NamedPrice>,
        lastTrustedPrice: Double?
    ): PriceOracleResult {
        try {
            val consensus = validateBitcoinConsensus(usdPrices, lastTrustedPrice)
            return PriceOracleResult(
                price = consensus.first,
                source = PriceOracleSource.DIRECT_USD,
                agreeingFeedNames = consensus.second.map { it.feedName }
            )
        } catch (error: PriceOracleException) {
            if (error.quarantinesPrice) throw error
        }

        val peg = validateUsdtPeg(pegPrices)
        val normalized = usdtPrices.map {
            NamedPrice(it.feedName, it.value * peg.first)
        }
        val consensus = validateBitcoinConsensus(normalized, lastTrustedPrice)
        return PriceOracleResult(
            price = consensus.first,
            source = PriceOracleSource.NORMALIZED_USDT,
            agreeingFeedNames = consensus.second.map { it.feedName },
            usdtUsd = peg.first
        )
    }

    fun validateBitcoinConsensus(
        prices: List<NamedPrice>,
        lastTrustedPrice: Double?
    ): Pair<Double, List<NamedPrice>> {
        val plausible = prices.filter { isPlausibleBitcoinPrice(it.value) }
        val initialMedian = median(plausible.map { it.value })
            ?: throw PriceOracleException("No plausible BTC/USD prices were returned")
        val agreeing = plausible.filter {
            relativeDeviation(it.value, initialMedian) <= MAXIMUM_FEED_DEVIATION_RATIO
        }
        if (agreeing.size < MINIMUM_AGREEING_FEEDS) {
            throw PriceOracleException(
                "BTC/USD consensus requires at least $MINIMUM_AGREEING_FEEDS agreeing feeds; got ${agreeing.size}"
            )
        }
        val acceptedMedian = median(agreeing.map { it.value })
            ?: throw PriceOracleException("Agreeing BTC/USD feeds produced no median")
        if (lastTrustedPrice != null && isPlausibleBitcoinPrice(lastTrustedPrice)) {
            val move = relativeDeviation(acceptedMedian, lastTrustedPrice)
            if (move > MAXIMUM_MEDIAN_MOVE_RATIO) {
                throw PriceOracleException(
                    "BTC/USD median moved ${"%.2f".format(move * 100)}% from the last trusted price",
                    quarantinesPrice = true
                )
            }
        }
        return acceptedMedian to agreeing
    }

    fun validateUsdtPeg(prices: List<NamedPrice>): Pair<Double, List<NamedPrice>> {
        val nearDollar = prices.filter {
            it.value.isFinite() && abs(it.value - 1.0) <= MAXIMUM_USDT_PEG_DEVIATION_FROM_DOLLAR
        }
        val initialMedian = median(nearDollar.map { it.value })
            ?: throw PriceOracleException("No valid USDT/USD peg prices were returned")
        val agreeing = nearDollar.filter {
            relativeDeviation(it.value, initialMedian) <= MAXIMUM_PEG_FEED_DEVIATION_RATIO
        }
        if (agreeing.size < MINIMUM_AGREEING_PEG_FEEDS) {
            throw PriceOracleException(
                "USDT fallback requires at least $MINIMUM_AGREEING_PEG_FEEDS agreeing peg feeds; got ${agreeing.size}"
            )
        }
        return (median(agreeing.map { it.value })
            ?: throw PriceOracleException("Agreeing USDT/USD feeds produced no median")) to agreeing
    }

    fun isPlausibleBitcoinPrice(price: Double): Boolean =
        price.isFinite() && price in MINIMUM_BITCOIN_USD..MAXIMUM_BITCOIN_USD

    fun median(values: List<Double>): Double? {
        if (values.isEmpty()) return null
        val sorted = values.sorted()
        val midpoint = sorted.size / 2
        return if (sorted.size % 2 == 0) {
            sorted[midpoint - 1] + (sorted[midpoint] - sorted[midpoint - 1]) / 2.0
        } else {
            sorted[midpoint]
        }
    }

    private fun relativeDeviation(value: Double, reference: Double): Double {
        if (!value.isFinite() || !reference.isFinite() || reference <= 0) return Double.POSITIVE_INFINITY
        return abs(value - reference) / reference
    }
}
