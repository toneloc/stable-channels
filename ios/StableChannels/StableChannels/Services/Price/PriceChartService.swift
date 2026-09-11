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
                guard let close = Double(closeStr) else { return nil }

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
                      let close = parseDouble(4) else {
                    return nil
                }

                let volume = parseDouble(6)

                return (date, open, high, low, close, volume)
            }
        } catch {
            return nil
        }
    }
}
