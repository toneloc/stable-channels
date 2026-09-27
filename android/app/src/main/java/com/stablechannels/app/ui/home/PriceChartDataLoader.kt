package com.stablechannels.app.ui.home

import com.stablechannels.app.AppState
import com.stablechannels.app.models.PriceRecord
import com.stablechannels.app.services.DatabaseService
import java.text.SimpleDateFormat
import java.util.Locale
import java.util.TimeZone
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Handles data fetching and parsing for the price chart from DatabaseService. */
object PriceChartDataLoader {

    suspend fun loadPriceData(
        databaseService: DatabaseService?,
        appState: AppState,
    ): Pair<List<PriceRecord>, List<PriceRecord>> =
        withContext(Dispatchers.IO) {
            val hourly = databaseService?.getPriceHistory(24 * 30) ?: emptyList()
            val dailyPrices = databaseService?.getDailyPrices(99999) ?: emptyList()
            val fmt =
                SimpleDateFormat("yyyy-MM-dd", Locale.US).apply {
                    timeZone = TimeZone.getTimeZone("UTC")
                }
            val daily =
                dailyPrices
                    .mapNotNull { d ->
                        val date =
                            try {
                                fmt.parse(d.date)
                            } catch (_: Exception) {
                                null
                            } ?: return@mapNotNull null
                        val ts = date.time / 1000
                        PriceRecord(id = ts, price = d.close, source = "daily", timestamp = ts)
                    }
                    .sortedBy { it.timestamp }

            hourly to daily
        }
}
