package com.stablechannels.app.services

import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.PriceFeedConfig
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.util.Date
import java.util.concurrent.TimeUnit

class PriceService {

    private val client = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private val _currentPrice = MutableStateFlow(0.0)
    val currentPrice: StateFlow<Double> = _currentPrice

    private val _lastUpdate = MutableStateFlow(Date(0))
    val lastUpdate: StateFlow<Date> = _lastUpdate

    private var refreshJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO)

    @Volatile
    private var isUpdating = false

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
            val prices = coroutineScope {
                Constants.DEFAULT_PRICE_FEEDS.map { feed ->
                    async { fetchSingleFeed(feed) }
                }.mapNotNull { it.await() }
            }
            val med = median(prices)
            if (med > 0) {
                _currentPrice.value = med
                _lastUpdate.value = Date()
            }
        } finally {
            isUpdating = false
        }
    }

    private suspend fun fetchSingleFeed(feed: PriceFeedConfig): Double? {
        val url = feed.urlFormat
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD")

        repeat(Constants.PRICE_FETCH_MAX_RETRIES) {
            try {
                val request = Request.Builder().url(url).build()
                val response = withContext(Dispatchers.IO) {
                    client.newCall(request).execute()
                }
                val body = response.body?.string() ?: return null
                val json = JSONObject(body)
                return extractPrice(json, feed.jsonPath)
            } catch (_: Exception) {
                delay(Constants.PRICE_FETCH_RETRY_DELAY_MS)
            }
        }
        return null
    }

    private fun extractPrice(json: JSONObject, path: List<String>): Double? {
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

    companion object {
        fun median(values: List<Double>): Double {
            if (values.isEmpty()) return 0.0
            val sorted = values.sorted()
            val mid = sorted.size / 2
            return if (sorted.size % 2 == 0) {
                (sorted[mid - 1] + sorted[mid]) / 2.0
            } else {
                sorted[mid]
            }
        }
    }
}
