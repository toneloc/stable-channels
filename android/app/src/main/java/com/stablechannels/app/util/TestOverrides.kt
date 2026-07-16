package com.stablechannels.app.util

import android.content.Context
import android.util.Log
import com.stablechannels.app.BuildConfig
import org.json.JSONObject
import java.io.File

/**
 * Debug-only endpoint overrides for E2E testing against the regtest harness
 * (e2e/harness). Release builds NEVER read the file (BuildConfig.DEBUG gate).
 *
 * A test rig pushes a JSON file after the harness boots (the LSP node id is
 * only known then — that's why this is runtime, not build config):
 *
 *   adb push test_config.json \
 *     /sdcard/Android/data/com.stablechannels.app/files/test_config.json
 *
 * Shape (all keys optional; absent = production value):
 * {
 *   "network": "regtest",
 *   "primary_chain_url": "http://10.0.2.2:30000",
 *   "fallback_chain_url": "http://10.0.2.2:30000",
 *   "lsp_pubkey": "<ldk-server node id>",
 *   "lsp_address": "10.0.2.2:9735",
 *   "push_register_url": "http://10.0.2.2:9737/register-push",
 *   "channel_exists_url": "http://10.0.2.2:9737/channel-exists",
 *   "price_feed_base": "http://10.0.2.2:9737"
 * }
 *
 * "price_feed_base" replaces ALL five price feeds with the harness's
 * /feeds/<name> endpoints (same JSON shapes as the real feeds).
 */
object TestOverrides {
    private const val TAG = "TestOverrides"
    private const val FILE_NAME = "test_config.json"

    @Volatile private var initialized = false

    @Volatile var network: String? = null; private set
    @Volatile var primaryChainUrl: String? = null; private set
    @Volatile var fallbackChainUrl: String? = null; private set
    @Volatile var lspPubkey: String? = null; private set
    @Volatile var lspAddress: String? = null; private set
    @Volatile var pushRegisterUrl: String? = null; private set
    @Volatile var channelExistsUrl: String? = null; private set
    @Volatile var priceFeedBase: String? = null; private set

    val active: Boolean get() = network != null || lspPubkey != null || primaryChainUrl != null

    fun init(context: Context) {
        if (initialized) return
        initialized = true
        if (!BuildConfig.DEBUG) return
        try {
            val f = File(context.getExternalFilesDir(null), FILE_NAME)
            if (!f.exists()) return
            val json = JSONObject(f.readText())
            fun opt(key: String) = json.optString(key).takeIf { it.isNotBlank() }
            network = opt("network")
            primaryChainUrl = opt("primary_chain_url")
            fallbackChainUrl = opt("fallback_chain_url")
            lspPubkey = opt("lsp_pubkey")
            lspAddress = opt("lsp_address")
            pushRegisterUrl = opt("push_register_url")
            channelExistsUrl = opt("channel_exists_url")
            priceFeedBase = opt("price_feed_base")
            Log.w(TAG, "E2E overrides ACTIVE: network=$network lsp=$lspAddress chain=$primaryChainUrl")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to load $FILE_NAME — using production endpoints", e)
        }
    }

    /** Harness-served replacements for the five production price feeds. */
    fun priceFeeds(base: String): List<PriceFeedConfig> = listOf(
        PriceFeedConfig("Bitstamp", "$base/feeds/bitstamp", listOf("last")),
        PriceFeedConfig("CoinGecko", "$base/feeds/coingecko", listOf("bitcoin", "usd")),
        PriceFeedConfig("Kraken", "$base/feeds/kraken", listOf("result", "XXBTZUSD", "c")),
        PriceFeedConfig("Coinbase", "$base/feeds/coinbase", listOf("data", "amount")),
        PriceFeedConfig("Blockchain.com", "$base/feeds/blockchain", listOf("USD", "last"))
    )
}
