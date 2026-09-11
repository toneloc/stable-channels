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
}
