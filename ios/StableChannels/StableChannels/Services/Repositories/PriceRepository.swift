import Foundation

final class PriceRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    func recordPrice(_ price: Double, source: String? = nil) throws {
        try rawSQL.execute(
            "INSERT INTO price_history (price, source) VALUES (?, ?)",
            params: [.real(price), source.map { .text($0) } ?? .null]
        )
    }

    func backfillHourlyPrices(_ prices: [(timestamp: Int64, price: Double)]) throws -> Int {
        guard !prices.isEmpty else { return 0 }
        return try rawSQL.inTransaction(mode: "DEFERRED") {
            var count = 0
            for (ts, price) in prices {
                let existing = try rawSQL.query(
                    "SELECT 1 FROM price_history WHERE timestamp BETWEEN ? AND ? LIMIT 1",
                    params: [.integer(ts - 1800), .integer(ts + 1800)]
                )
                if existing.isEmpty {
                    try rawSQL.execute(
                        "INSERT INTO price_history (price, source, timestamp) VALUES (?, 'kraken_ohlc', ?)",
                        params: [.real(price), .integer(ts)]
                    )
                    count += 1
                }
            }
            return count
        }
    }

    func getOldestPriceHistoryTimestamp() throws -> Int64? {
        let rows = try rawSQL.query("SELECT MIN(timestamp) FROM price_history")
        return rows.first?.optInt64(0)
    }

    func getPriceHistory(hours: UInt32) throws -> [PriceRecord] {
        let cutoff = Int64(Date().timeIntervalSince1970) - Int64(hours) * 3600
        let sql = """
            SELECT id, price, source, timestamp FROM price_history
            WHERE timestamp > ? ORDER BY timestamp ASC
        """
        let rows = try rawSQL.query(sql, params: [.integer(cutoff)])
        return rows.map { row in
            PriceRecord(
                id: row.int64(0),
                price: row.double(1),
                source: row.optString(2),
                timestamp: row.int64(3)
            )
        }
    }

    func getDailyPrices(days: UInt32) throws -> [DailyPriceRecord] {
        let calendar = Calendar.current
        let cutoffDate = calendar.date(byAdding: .day, value: -Int(days), to: Date()) ?? Date()
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        let cutoffStr = formatter.string(from: cutoffDate)

        let sql = """
            SELECT date, open, high, low, close, volume FROM daily_prices
            WHERE date >= ? ORDER BY date ASC
        """
        let rows = try rawSQL.query(sql, params: [.text(cutoffStr)])
        return rows.map { row in
            DailyPriceRecord(
                date: row.string(0),
                open: row.double(1),
                high: row.double(2),
                low: row.double(3),
                close: row.double(4),
                volume: row.optDouble(5)
            )
        }
    }

    func recordDailyPrice(
        date: String, open: Double, high: Double, low: Double,
        close: Double, volume: Double?, source: String? = nil
    ) throws {
        try rawSQL.execute(
            "INSERT OR REPLACE INTO daily_prices (date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params: [
                .text(date), .real(open), .real(high), .real(low), .real(close),
                volume.map { .real($0) } ?? .null,
                source.map { .text($0) } ?? .null
            ]
        )
    }

    func bulkInsertDailyPrices(_ prices: [(String, Double, Double, Double, Double, Double?)]) throws -> Int {
        guard !prices.isEmpty else { return 0 }
        return try rawSQL.inTransaction(mode: "DEFERRED") {
            for (date, open, high, low, close, volume) in prices {
                try rawSQL.execute(
                    "INSERT OR IGNORE INTO daily_prices (date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, 'seed')",
                    params: [
                        .text(date), .real(open), .real(high), .real(low), .real(close),
                        volume.map { .real($0) } ?? .null
                    ]
                )
            }
            return prices.count
        }
    }

    func getOldestDailyPriceDate() throws -> String? {
        let rows = try rawSQL.query("SELECT date FROM daily_prices ORDER BY date ASC LIMIT 1", params: [])
        return rows.first?.optString(0)
    }
}
