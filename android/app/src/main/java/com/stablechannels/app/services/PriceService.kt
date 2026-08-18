package com.stablechannels.app.services

import android.content.Context
import android.util.Log
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.NamedPrice
import com.stablechannels.app.util.PriceFeedConfig
import com.stablechannels.app.util.PriceOracle
import com.stablechannels.app.util.PriceOracleAnchorStore
import com.stablechannels.app.util.PriceOracleException
import com.stablechannels.app.util.PriceOracleSource
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import java.util.Date
import java.util.concurrent.TimeUnit

class PriceService(private val appContext: Context? = null) {

    private val client = OkHttpClient.Builder()
        .connectTimeout(Constants.PRICE_FETCH_TIMEOUT_SECS, TimeUnit.SECONDS)
        .readTimeout(Constants.PRICE_FETCH_TIMEOUT_SECS, TimeUnit.SECONDS)
        .callTimeout(Constants.PRICE_FETCH_TIMEOUT_SECS, TimeUnit.SECONDS)
        .build()

    private val _currentPrice = MutableStateFlow(0.0)
    val currentPrice: StateFlow<Double> = _currentPrice

    private val _lastUpdate = MutableStateFlow(Date(0))
    val lastUpdate: StateFlow<Date> = _lastUpdate

    private val _accountingPrice = MutableStateFlow(0.0)
    val accountingPrice: StateFlow<Double> = _accountingPrice

    private val _activeSource = MutableStateFlow<PriceOracleSource?>(null)
    val activeSource: StateFlow<PriceOracleSource?> = _activeSource

    @Volatile
    private var isQuarantined = false

    /** Returns true if the price was last updated more than [maxAgeSecs] seconds ago. */
    fun isPriceStale(maxAgeSecs: Long = PriceOracle.MAXIMUM_TRUSTED_PRICE_AGE_SECS): Boolean {
        val ageMs = System.currentTimeMillis() - _lastUpdate.value.time
        return ageMs > maxAgeSecs * 1000
    }

    /** Re-check freshness at the point of use so money movement never relies on a stale flow value. */
    fun currentAccountingPrice(): Double =
        _currentPrice.value.takeIf { it > 0.0 && !isQuarantined && !isPriceStale() } ?: 0.0

    private var refreshJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO)

    @Volatile
    private var isUpdating = false

    fun seedPrice(price: Double) {
        if (_currentPrice.value <= 0.0 && price > 0.0) {
            _currentPrice.value = price
        }
    }

    fun startAutoRefresh(intervalSecs: Long = Constants.PRICE_CACHE_REFRESH_SECS) {
        refreshJob?.cancel()
        refreshJob = scope.launch {
            while (isActive) {
                fetchPrice()
                delay(intervalSecs * 1000)
            }
        }
    }

    fun stopAutoRefresh() {
        refreshJob?.cancel()
        refreshJob = null
    }

    suspend fun fetchPrice() {
        if (isUpdating) return
        isUpdating = true
        try {
            val lastTrustedPrice = _currentPrice.value.takeIf {
                it > 0 && !isPriceStale()
            }
            val usdPrices = fetchFeeds(PriceOracle.DIRECT_USD_FEEDS)
            val result = try {
                PriceOracle.resolve(usdPrices, emptyList(), emptyList(), lastTrustedPrice)
            } catch (error: PriceOracleException) {
                if (error.quarantinesPrice) throw error
                Log.w(TAG, "Direct USD unavailable: ${error.message}; trying USDT fallback")
                coroutineScope {
                    val usdtPrices = async { fetchFeeds(PriceOracle.BITCOIN_USDT_FEEDS) }
                    val pegPrices = async { fetchFeeds(PriceOracle.USDT_USD_FEEDS) }
                    PriceOracle.resolve(
                        emptyList(),
                        usdtPrices.await(),
                        pegPrices.await(),
                        lastTrustedPrice
                    )
                }
            }

            _currentPrice.value = result.price
            _lastUpdate.value = Date()
            _accountingPrice.value = result.price
            _activeSource.value = result.source
            isQuarantined = false
            // Persist the accepted price so the background stability service inherits the
            // large-move circuit breaker (mirrors the iOS app-group anchor).
            appContext?.let {
                PriceOracleAnchorStore.save(it, result.price, _lastUpdate.value.time)
            }
            Log.d(
                TAG,
                "Accepted ${result.source} price from ${result.agreeingFeedNames.size} feeds" +
                    (result.usdtUsd?.let { ", USDT/USD=$it" } ?: "")
            )
        } catch (error: Exception) {
            if ((error as? PriceOracleException)?.quarantinesPrice == true) {
                isQuarantined = true
            }
            _accountingPrice.value = if (!isQuarantined && !isPriceStale()) {
                _currentPrice.value
            } else {
                0.0
            }
            Log.w(TAG, "Rejected price refresh: ${error.message}")
        } finally {
            isUpdating = false
        }
    }

    private suspend fun fetchFeeds(feeds: List<PriceFeedConfig>): List<NamedPrice> = coroutineScope {
        feeds.map { feed ->
            async {
                fetchSingleFeed(feed)?.let { NamedPrice(feed.name, it) }
            }
        }.mapNotNull { it.await() }
    }

    private suspend fun fetchSingleFeed(feed: PriceFeedConfig): Double? {
        val url = feed.urlFormat
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD")

        return try {
            val request = Request.Builder().url(url).build()
            val response = withContext(Dispatchers.IO) { client.newCall(request).execute() }
            response.use {
                if (!it.isSuccessful) {
                    Log.w(TAG, "${feed.name} failed: HTTP ${it.code}")
                    return null
                }
                val body = it.body?.string() ?: return null
                val json = JSONTokener(body).nextValue()
                val price = extractPrice(json, feed.jsonPath)
                if (price == null) {
                    Log.w(TAG, "${feed.name} failed: invalid response path")
                } else {
                    Log.d(TAG, "${feed.name} succeeded")
                }
                price
            }
        } catch (error: Exception) {
            Log.w(TAG, "${feed.name} failed: ${error.message}")
            null
        }
    }

    private fun extractPrice(json: Any, path: List<String>): Double? {
        var current: Any = json
        for (key in path) {
            current = when (current) {
                is JSONObject -> current.opt(key) ?: return null
                is JSONArray -> current.opt(key.toIntOrNull() ?: return null) ?: return null
                else -> return null
            }
        }
        return when (current) {
            is Double -> current
            is Int -> current.toDouble()
            is Long -> current.toDouble()
            is String -> current.toDoubleOrNull()
            is JSONArray -> {
                // Kraken returns ["price", "volume"] - take first
                val first = current.opt(0)
                when (first) {
                    is String -> first.toDoubleOrNull()
                    is Double -> first
                    else -> null
                }
            }
            else -> null
        }
    }

    suspend fun fetchKrakenOHLC(since: Long? = null): List<Pair<Long, Double>> {
        val sinceTs = since ?: (System.currentTimeMillis() / 1000 - 30 * 24 * 3600)
        val url = "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=60&since=$sinceTs"
        return try {
            val request = Request.Builder().url(url).build()
            val response = withContext(Dispatchers.IO) { client.newCall(request).execute() }
            val body = response.body?.string() ?: return emptyList()
            val json = JSONObject(body)
            val result = json.optJSONObject("result") ?: return emptyList()
            val xxbtzusd = result.optJSONArray("XXBTZUSD") ?: return emptyList()
            val candles = mutableListOf<Pair<Long, Double>>()
            for (i in 0 until xxbtzusd.length()) {
                val candle = xxbtzusd.optJSONArray(i) ?: continue
                val ts = candle.optLong(0)
                val close = candle.optString(4).toDoubleOrNull() ?: continue
                if (ts > 0 && close > 0) candles.add(ts to close)
            }
            candles.sortedBy { it.first }
        } catch (_: Exception) {
            emptyList()
        }
    }

    companion object {
        private const val TAG = "PriceOracle"

        fun median(values: List<Double>): Double {
            return PriceOracle.median(values) ?: 0.0
        }
    }
}
