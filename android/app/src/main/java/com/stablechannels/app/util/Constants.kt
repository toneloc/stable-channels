package com.stablechannels.app.util

import android.content.Context
import org.lightningdevkit.ldknode.Network
import java.io.File

object Constants {
    const val SATS_IN_BTC: Long = 100_000_000L
    const val STABLE_CHANNEL_TLV_TYPE: Long = 13_377_331L
    const val TRADE_MESSAGE_TYPE = "TRADE_V1"
    const val SYNC_MESSAGE_TYPE = "SYNC_V1"

    // Overridable via TestOverrides (debug builds only) for E2E regtest runs.
    val DEFAULT_NETWORK: String get() = TestOverrides.network ?: "bitcoin"
    val LDK_NETWORK: Network get() = when (DEFAULT_NETWORK.lowercase()) {
        "regtest" -> Network.REGTEST
        "signet" -> Network.SIGNET
        "testnet" -> Network.TESTNET
        else -> Network.BITCOIN
    }
    const val DEFAULT_USER_ALIAS = "user"
    const val DEFAULT_USER_PORT = 9736
    const val DEFAULT_LSP_ALIAS = "lsp"
    const val DEFAULT_LSP_PORT = 9735

    val LSP_PUSH_REGISTER_URL: String get() =
        TestOverrides.pushRegisterUrl ?: "https://stablechannels.com/api/register-push"
    val LSP_CHANNEL_EXISTS_URL: String get() =
        TestOverrides.channelExistsUrl ?: "https://stablechannels.com/api/channel-exists"
    const val PRIVACY_POLICY_URL = "https://stablechannels.com/privacy.html"

    val PRIMARY_CHAIN_URL: String get() =
        TestOverrides.primaryChainUrl ?: "https://blockstream.info/api"
    val FALLBACK_CHAIN_URL: String get() =
        TestOverrides.fallbackChainUrl ?: "https://mempool.space/api"
    val DEFAULT_LSP_PUBKEY: String get() =
        TestOverrides.lspPubkey ?: "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    val DEFAULT_LSP_ADDRESS: String get() =
        TestOverrides.lspAddress ?: "stablechannels.com:9735"

    const val PRICE_CACHE_REFRESH_SECS: Long = 15
    const val PRICE_FETCH_TIMEOUT_SECS: Long = 3
    /** Longer budget for the ~30-day hourly OHLC chart backfill, which is a much larger download
     *  than a single-price ticker and must not share the short per-feed ticker timeout. */
    const val CHART_FETCH_TIMEOUT_SECS: Long = 30

    // E2E override shortens both (regtest blocks are on demand; 120s syncs
    // just add dead time to test runs).
    val ONCHAIN_WALLET_SYNC_INTERVAL_SECS: Long get() =
        TestOverrides.syncIntervalSecs ?: 120
    val LIGHTNING_WALLET_SYNC_INTERVAL_SECS: Long get() =
        TestOverrides.syncIntervalSecs ?: 60
    const val FEE_RATE_CACHE_UPDATE_INTERVAL_SECS: Long = 1200

    const val INVOICE_EXPIRY_SECS: Int = 3600
    const val BALANCE_UPDATE_INTERVAL_SECS: Long = 30
    // Deposit detection shares this timer. Keep production at 60s while
    // allowing regtest to observe mined deposits without passive waiting.
    val STABILITY_CHECK_INTERVAL_SECS: Long get() =
        TestOverrides.syncIntervalSecs ?: 60
    val SPLICE_CONFIRMATION_POLL_INTERVAL_SECS: Long get() =
        TestOverrides.syncIntervalSecs ?: 30
    val ONCHAIN_DEPOSIT_POLL_INTERVAL_SECS: Long get() =
        TestOverrides.syncIntervalSecs ?: 10
    const val MAX_RISK_LEVEL = 100
    const val STABILITY_THRESHOLD_PERCENT: Double = 0.1
    const val STABILITY_THRESHOLD_USD: Double = 0.25
    const val STABILITY_PAYMENT_COOLDOWN_SECS: Long = 120

    // A stability payment may only be sent when the Lightning wallet synced to chain within
    // this window (two 60s background sync intervals, so one missed tick is tolerated).
    // Keep in sync with STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS in src/constants.rs.
    const val STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS: Long = 120
    const val MIN_DISPLAY_USD: Double = 2.0
    const val MAX_CHANNEL_USD: Double = 100.0
    /** Stable-channel trade fee paid to the LSP as the TRADE_V1 keysend amount. */
    const val STABLE_CHANNEL_TRADE_FEE_RATE: Double = 0.01
    const val LIGHTNING_DEFAULT_FORWARDING_FEE_BASE_MSAT: Long = 1_000L
    const val LIGHTNING_DEFAULT_FORWARDING_FEE_PROPORTIONAL_MILLIONTHS: Long = 0L
    const val ESTIMATED_ONCHAIN_SEND_VBYTES: Long = 140L
    const val ESTIMATED_ONCHAIN_SEND_ALL_VBYTES: Long = 250L
    const val ESTIMATED_CHANNEL_CLOSE_VBYTES: Long = 180L
    const val DEFAULT_CHANNEL_LIFETIME: Int = 2016
    const val MAX_PAYMENT_SIZE_MSAT: Long = 100_000_000_000L
    const val CHANNEL_OVER_PROVISIONING_PPM: Int = 1_000_000

    // E2E override hook: when TestOverrides supplies a local feed base, all price feeds
    // route there; production uses the PriceOracle feed sets from main.
    val DEFAULT_PRICE_FEEDS: List<PriceFeedConfig> get() =
        TestOverrides.priceFeedBase?.let { TestOverrides.priceFeeds(it) } ?: PriceOracle.DIRECT_USD_FEEDS
    // The USDT fallback must be silenced under the E2E override: real exchange prices
    // diverge arbitrarily from the harness's mocked price, so one transient direct-feed
    // miss would resolve real-world quotes against the mocked lastTrustedPrice, trip the
    // large-move guard, and quarantine the price — blocking every send mid-suite. An
    // empty fallback fails non-quarantining and the mocked price stays trusted.
    val FALLBACK_USDT_PRICE_FEEDS: List<PriceFeedConfig> get() =
        if (TestOverrides.priceFeedBase != null) emptyList() else PriceOracle.BITCOIN_USDT_FEEDS
    val USDT_USD_PRICE_FEEDS: List<PriceFeedConfig> get() =
        if (TestOverrides.priceFeedBase != null) emptyList() else PriceOracle.USDT_USD_FEEDS

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
