package com.stablechannels.app.util

import android.content.Context
import java.io.File

object Constants {
    const val SATS_IN_BTC: Long = 100_000_000L
    const val STABLE_CHANNEL_TLV_TYPE: Long = 13_377_331L
    const val TRADE_MESSAGE_TYPE = "TRADE_V1"
    const val SYNC_MESSAGE_TYPE = "SYNC_V1"

    const val DEFAULT_NETWORK = "bitcoin"
    const val DEFAULT_USER_ALIAS = "user"
    const val DEFAULT_USER_PORT = 9736
    const val DEFAULT_LSP_ALIAS = "lsp"
    const val DEFAULT_LSP_PORT = 9735

    const val LSP_PUSH_REGISTER_URL = "https://stablechannels.com/api/register-push"

    const val PRIMARY_CHAIN_URL = "https://blockstream.info/api"
    const val FALLBACK_CHAIN_URL = "https://mempool.space/api"
    const val DEFAULT_LSP_PUBKEY = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    const val DEFAULT_LSP_ADDRESS = "34.198.44.89:9735"
    const val DEFAULT_GATEWAY_PUBKEY = "03da1c27ca77872ac5b3e568af30673e599a47a5e4497f85c7b5da42048807b3ed"
    const val DEFAULT_GATEWAY_ADDRESS = "213.174.156.80:9735"

    const val PRICE_CACHE_REFRESH_SECS: Long = 5
    const val PRICE_FETCH_RETRY_DELAY_MS: Long = 300
    const val PRICE_FETCH_MAX_RETRIES = 3

    const val ONCHAIN_WALLET_SYNC_INTERVAL_SECS: Long = 120
    const val LIGHTNING_WALLET_SYNC_INTERVAL_SECS: Long = 60
    const val FEE_RATE_CACHE_UPDATE_INTERVAL_SECS: Long = 1200

    const val INVOICE_EXPIRY_SECS: Int = 3600
    const val BALANCE_UPDATE_INTERVAL_SECS: Long = 30
    const val STABILITY_CHECK_INTERVAL_SECS: Long = 60
    const val MAX_RISK_LEVEL = 100
    const val STABILITY_THRESHOLD_PERCENT: Double = 0.1
    const val STABILITY_THRESHOLD_USD: Double = 0.25
    const val STABILITY_PAYMENT_COOLDOWN_SECS: Long = 120
    const val MIN_DISPLAY_USD: Double = 2.0
    const val MAX_CHANNEL_USD: Double = 100.0
    const val DEFAULT_CHANNEL_LIFETIME: Int = 2016
    const val MAX_PAYMENT_SIZE_MSAT: Long = 100_000_000_000L
    const val CHANNEL_OVER_PROVISIONING_PPM: Int = 1_000_000

    val DEFAULT_PRICE_FEEDS = listOf(
        PriceFeedConfig("Bitstamp", "https://www.bitstamp.net/api/v2/ticker/btc{currency_lc}/", listOf("last")),
        PriceFeedConfig("CoinGecko", "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies={currency_lc}", listOf("bitcoin", "usd")),
        PriceFeedConfig("Kraken", "https://api.kraken.com/0/public/Ticker?pair=XBT{currency}", listOf("result", "XXBTZUSD", "c")),
        PriceFeedConfig("Coinbase", "https://api.coinbase.com/v2/prices/BTC-{currency}/spot", listOf("data", "amount")),
        PriceFeedConfig("Blockchain.com", "https://blockchain.info/ticker", listOf("{currency}", "last"))
    )

    object RGSServer {
        const val BITCOIN = "https://rapidsync.lightningdevkit.org/snapshot/"
        const val SIGNET = "https://mutinynet-flow.eldamar.icu/v1/rgs/snapshot/"
        const val TESTNET = "https://rapidsync.lightningdevkit.org/testnet/snapshot/"
    }

    fun userDataDir(context: Context): File {
        val dir = File(context.filesDir, "stablechannels/user")
        if (!dir.exists()) dir.mkdirs()
        return dir
    }
}

data class PriceFeedConfig(
    val name: String,
    val urlFormat: String,
    val jsonPath: List<String>
)
