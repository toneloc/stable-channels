package com.stablechannels.app.services

import com.stablechannels.app.models.DailyPriceRecord

/**
 * Interface defining contract for fetching historical Bitcoin price chart data (Kraken OHLC).
 */
interface PriceChartFetcher {
    /**
     * Fetch hourly OHLC candles from Kraken.
     * Returns list of Pair(unix_timestamp, close_price).
     */
    suspend fun fetchKrakenHourlyOHLC(since: Long? = null): List<Pair<Long, Double>>

    /**
     * Fetch daily OHLC candles from Kraken (up to 720 days).
     * Returns list of DailyPriceRecord.
     */
    suspend fun fetchKrakenDailyOHLC(since: Long? = null): List<DailyPriceRecord>
}
