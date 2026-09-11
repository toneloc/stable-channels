package com.stablechannels.app.services

import com.stablechannels.app.models.DailyPriceRecord
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.TimeUnit

/**
 * Service dedicated to fetching historical Bitcoin price chart data (Kraken OHLC).
 */
class PriceChartService(
    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(15, TimeUnit.SECONDS)
        .build()
) : PriceChartFetcher {

    /**
     * Fetch hourly OHLC candles from Kraken for the last ~30 days.
     * Returns list of Pair(unix_timestamp, close_price).
     */
    override suspend fun fetchKrakenHourlyOHLC(since: Long?): List<Pair<Long, Double>> {
        val sinceTs = since ?: (System.currentTimeMillis() / 1000 - 30 * 24 * 3600)
        val url = "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=60&since=$sinceTs"
        return try {
            val request = Request.Builder().url(url).build()
            val response = withContext(Dispatchers.IO) { client.newCall(request).execute() }
            val body = response.body?.string() ?: return emptyList()
            val json = JSONObject(body)
            val result = json.optJSONObject("result") ?: return emptyList()
            val xxbtzusd = result.optJSONArray("XXBTZUSD") ?: result.optJSONArray("XBTUSD") ?: return emptyList()
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

    /**
     * Fetch daily OHLC candles from Kraken (up to 720 days).
     * Returns list of DailyPriceRecord.
     */
    override suspend fun fetchKrakenDailyOHLC(since: Long?): List<DailyPriceRecord> {
        val sinceTs = since ?: (System.currentTimeMillis() / 1000 - 720 * 24 * 3600)
        val url = "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=1440&since=$sinceTs"
        return try {
            val request = Request.Builder().url(url).build()
            val response = withContext(Dispatchers.IO) { client.newCall(request).execute() }
            val body = response.body?.string() ?: return emptyList()
            val json = JSONObject(body)
            val result = json.optJSONObject("result") ?: return emptyList()
            val candlesArray = result.optJSONArray("XXBTZUSD") ?: result.optJSONArray("XBTUSD") ?: return emptyList()

            val fmt = SimpleDateFormat("yyyy-MM-dd", Locale.US).apply {
                timeZone = TimeZone.getTimeZone("UTC")
            }

            val records = mutableListOf<DailyPriceRecord>()
            for (i in 0 until candlesArray.length()) {
                val candle = candlesArray.optJSONArray(i) ?: continue
                val ts = candle.optLong(0)
                val open = candle.optString(1).toDoubleOrNull() ?: continue
                val high = candle.optString(2).toDoubleOrNull() ?: continue
                val low = candle.optString(3).toDoubleOrNull() ?: continue
                val close = candle.optString(4).toDoubleOrNull() ?: continue
                val volume = candle.optString(6).toDoubleOrNull()

                if (ts > 0) {
                    val dateStr = fmt.format(Date(ts * 1000))
                    records.add(
                        DailyPriceRecord(
                            date = dateStr,
                            open = open,
                            high = high,
                            low = low,
                            close = close,
                            volume = volume
                        )
                    )
                }
            }
            records.sortedBy { it.date }
        } catch (_: Exception) {
            emptyList()
        }
    }

    companion object {
        val shared = PriceChartService()
    }
}
