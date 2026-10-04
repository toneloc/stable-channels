import Foundation

/// Service dedicated to fetching historical Bitcoin price chart data (Kraken OHLC).
final class PriceChartService: PriceChartFetching, @unchecked Sendable {
    static let shared = PriceChartService()

    /// Longer-lived session for historical-chart backfill. The ~30-day hourly OHLC payload is far
    /// larger than a ticker response, so the short per-feed timeout would silently truncate it to an
    /// empty chart on a slow cellular link.
    private let chartSession: URLSession

    init(session: URLSession? = nil) {
        if let session {
            self.chartSession = session
        } else {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.timeoutIntervalForRequest = Constants.chartFetchTimeoutSecs
            configuration.timeoutIntervalForResource = Constants.chartFetchTimeoutSecs
            configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
            configuration.waitsForConnectivity = false
            self.chartSession = URLSession(configuration: configuration)
        }
    }

    /// Fetch hourly OHLC candles from Kraken for the last ~30 days.
    /// Returns array of (unix_timestamp, close_price), or nil on network/API error.
    func fetchKrakenHourlyOHLC(since: Int64? = nil) async -> [(timestamp: Int64, price: Double)]? {
        let sinceTs = since ?? (Int64(Date().timeIntervalSince1970) - 30 * 24 * 3600)
        guard let url = URL(string: "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=60&since=\(sinceTs)")
        else {
            return nil
        }

        do {
            let (data, response) = try await chartSession.data(from: url)
            guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
                return nil
            }
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                return nil
            }
            if let errors = json["error"] as? [Any], !errors.isEmpty {
                return nil
            }
            guard let result = json["result"] as? [String: Any] else {
                return nil
            }

            let candlesArray: [[Any]]? = (result["XXBTZUSD"] as? [[Any]]) ?? (result["XBTUSD"] as? [[Any]])
            guard let candles = candlesArray else {
                return []
            }

            return candles.compactMap { candle -> (Int64, Double)? in
                guard candle.count >= 5 else { return nil }
                let ts: Int64
                if let t = candle[0] as? Int64 {
                    ts = t
                } else if let t = candle[0] as? Int {
                    ts = Int64(t)
                } else if let t = candle[0] as? Double {
                    ts = Int64(t)
                } else {
                    return nil
                }

                let closeStr: String
                if let s = candle[4] as? String {
                    closeStr = s
                } else {
                    return nil
                }
                guard let close = Double(closeStr), ts > 0, close > 0 else { return nil }

                return (ts, close)
            }
        } catch {
            return nil
        }
    }

    /// Convenience wrapper for backward compatibility.
    func fetchKrakenOHLC(since: Int64? = nil) async -> [(timestamp: Int64, price: Double)]? {
        await fetchKrakenHourlyOHLC(since: since)
    }

    /// Fetch daily OHLC candles from Kraken (up to 720 days).
    /// Returns array of (date, open, high, low, close, volume), or nil on network/API error.
    func fetchKrakenDailyOHLC(since: Int64? = nil) async -> [(
        date: String,
        open: Double,
        high: Double,
        low: Double,
        close: Double,
        volume: Double?
    )]? {
        var urlString = "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=1440"
        if let since {
            urlString += "&since=\(since)"
        }
        guard let url = URL(string: urlString) else { return nil }

        do {
            let (data, response) = try await chartSession.data(from: url)
            guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
                return nil
            }
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                return nil
            }
            if let errors = json["error"] as? [Any], !errors.isEmpty {
                return nil
            }
            guard let result = json["result"] as? [String: Any] else {
                return nil
            }

            let candlesArray: [[Any]]? = (result["XXBTZUSD"] as? [[Any]]) ?? (result["XBTUSD"] as? [[Any]])
            guard let candles = candlesArray else { return [] }

            let dateFormatter = DateFormatter()
            dateFormatter.dateFormat = "yyyy-MM-dd"
            dateFormatter.timeZone = TimeZone(secondsFromGMT: 0)

            return candles.compactMap { candle -> (String, Double, Double, Double, Double, Double?)? in
                guard candle.count >= 5 else { return nil }
                let ts: Int64
                if let t = candle[0] as? Int64 {
                    ts = t
                } else if let t = candle[0] as? Int {
                    ts = Int64(t)
                } else if let t = candle[0] as? Double {
                    ts = Int64(t)
                } else {
                    return nil
                }

                let date = dateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(ts)))

                func parseDouble(_ index: Int) -> Double? {
                    guard index < candle.count else { return nil }
                    if let s = candle[index] as? String {
                        return Double(s)
                    } else if let d = candle[index] as? Double {
                        return d
                    }
                    return nil
                }

                guard let open = parseDouble(1),
                      let high = parseDouble(2),
                      let low = parseDouble(3),
                      let close = parseDouble(4),
                      ts > 0, open > 0, high > 0, low > 0, close > 0,
                      high >= low, open >= low, open <= high, close >= low, close <= high else {
                    return nil
                }

                let volume = parseDouble(6)

                return (date, open, high, low, close, volume)
            }
        } catch {
            return nil
        }
    }

    // MARK: - Backfill & Seeding

    private let backfillLock = NSLock()
    private var isBackfillingHourlyState: Bool = false
    private var isBackfillingDailyState: Bool = false

    private func tryAcquireHourlyBackfill() -> Bool {
        backfillLock.lock()
        defer { backfillLock.unlock() }
        guard !isBackfillingHourlyState else { return false }
        isBackfillingHourlyState = true
        return true
    }

    private func releaseHourlyBackfill() {
        backfillLock.lock()
        isBackfillingHourlyState = false
        backfillLock.unlock()
    }

    private func tryAcquireDailyBackfill() -> Bool {
        backfillLock.lock()
        defer { backfillLock.unlock() }
        guard !isBackfillingDailyState else { return false }
        isBackfillingDailyState = true
        return true
    }

    private func releaseDailyBackfill() {
        backfillLock.lock()
        isBackfillingDailyState = false
        backfillLock.unlock()
    }

    private static let dailyDateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        return formatter
    }()

    /// Fetch hourly candles from Kraken and backfill price_history for smooth 1W/1M charts.
    func backfillHourlyPrices(priceRepo: PriceRepository) async {
        guard tryAcquireHourlyBackfill() else { return }
        defer { releaseHourlyBackfill() }

        // Determine how far back we need data — up to 30 days
        let thirtyDaysAgo = Int64(Date().timeIntervalSince1970) - 30 * 24 * 3600
        let since: Int64
        if let oldest = try? priceRepo.getOldestPriceHistoryTimestamp(), oldest < thirtyDaysAgo {
            // Already have old enough data, just fill gaps from the newest record
            since = (try? priceRepo.getLatestPriceHistoryTimestamp()) ?? thirtyDaysAgo
        } else {
            since = thirtyDaysAgo
        }

        for attempt in 1...3 {
            guard let candles = await fetchKrakenHourlyOHLC(since: since) else {
                if attempt < 3 {
                    try? await Task.sleep(nanoseconds: UInt64(attempt) * 1_000_000_000)
                }
                continue
            }
            if !candles.isEmpty {
                do {
                    let count = try priceRepo.backfillHourlyPrices(candles)
                    if count > 0 {
                        print("[Chart] Backfilled \(count) hourly price points from Kraken")
                        await MainActor.run {
                            NotificationCenter.default.post(name: .priceHistoryUpdated, object: nil)
                        }
                    }
                } catch {
                    print("[Chart] Hourly backfill failed: \(error)")
                }
            }
            break
        }
    }

    /// Fetch daily candles from Kraken and backfill daily_prices for smooth 3M/6M/1Y/ALL charts.
    func backfillDailyPrices(priceRepo: PriceRepository) async {
        guard tryAcquireDailyBackfill() else { return }
        defer { releaseDailyBackfill() }

        // Determine how far back we need data — up to 720 days
        let sevenTwentyDaysAgo = Int64(Date().timeIntervalSince1970) - 720 * 24 * 3600
        let since: Int64

        if let latest = try? priceRepo.getLatestDailyPriceDate(),
           let date = Self.dailyDateFormatter.date(from: latest) {
            let latestTs = Int64(date.timeIntervalSince1970)
            since = max(latestTs - 86400, sevenTwentyDaysAgo)
        } else {
            since = sevenTwentyDaysAgo
        }

        for attempt in 1...3 {
            guard let candles = await fetchKrakenDailyOHLC(since: since) else {
                if attempt < 3 {
                    try? await Task.sleep(nanoseconds: UInt64(attempt) * 1_000_000_000)
                }
                continue
            }
            if !candles.isEmpty {
                do {
                    let count = try priceRepo.backfillDailyPrices(candles)
                    if count > 0 {
                        print("[Chart] Backfilled \(count) daily price points from Kraken")
                    }
                    await MainActor.run {
                        NotificationCenter.default.post(name: .priceHistoryUpdated, object: nil)
                    }
                } catch {
                    print("[Chart] Daily backfill failed: \(error)")
                }
            }
            break
        }
    }

    /// Seed historical daily prices from bundled seed data if database has not yet been seeded.
    func seedHistoricalPrices(priceRepo: PriceRepository) {
        let needsSeed: Bool
        do {
            if let oldest = try priceRepo.getOldestDailyPriceDate() {
                needsSeed = !oldest.hasPrefix("2013")
            } else {
                needsSeed = true
            }
        } catch {
            needsSeed = true
        }

        guard needsSeed else {
            print("[Chart] Historical prices already seeded")
            return
        }

        print("[Chart] Seeding historical price data (2013-present)...")
        do {
            let count = try priceRepo.bulkInsertDailyPrices(HistoricalPrices.seedPrices)
            print("[Chart] Seeded \(count) historical price records")
            if count > 0 {
                Task { @MainActor in
                    NotificationCenter.default.post(name: .priceHistoryUpdated, object: nil)
                }
            }
        } catch {
            print("[Chart] Failed to seed historical prices: \(error)")
        }
    }
}
