import Foundation

/// Protocol defining the contract for historical Bitcoin price chart data fetching (Kraken OHLC).
protocol PriceChartFetching: Sendable {
    /// Fetch hourly OHLC candles from Kraken.
    /// Returns array of (unix_timestamp, close_price), or nil on network/API error.
    func fetchKrakenHourlyOHLC(since: Int64?) async -> [(timestamp: Int64, price: Double)]?

    /// Fetch daily OHLC candles from Kraken.
    /// Returns array of (date_string, open, high, low, close, volume), or nil on network/API error.
    func fetchKrakenDailyOHLC(since: Int64?) async -> [(
        date: String,
        open: Double,
        high: Double,
        low: Double,
        close: Double,
        volume: Double?
    )]?

    /// Backfill hourly candles into the price repository.
    func backfillHourlyPrices(priceRepo: PriceRepository) async

    /// Backfill daily candles into the price repository.
    func backfillDailyPrices(priceRepo: PriceRepository) async

    /// Seed bundled historical daily prices into the price repository if needed.
    func seedHistoricalPrices(priceRepo: PriceRepository)
}
