import Foundation

/// Service dedicated to fetching historical Bitcoin price chart data (Kraken OHLC).
final class PriceChartService {
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
    /// Returns array of (unix_timestamp, close_price).
    func fetchKrakenOHLC(since: Int64? = nil) async -> [(timestamp: Int64, price: Double)] {
        let sinceTs = since ?? (Int64(Date().timeIntervalSince1970) - 30 * 24 * 3600)
        guard let url = URL(string: "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=60&since=\(sinceTs)")
        else {
            return []
        }

        do {
            let (data, response) = try await chartSession.data(from: url)
            guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
                return []
            }
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let result = json["result"] as? [String: Any],
                  let candles = result["XXBTZUSD"] as? [[Any]] else {
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
            return []
        }
    }
}
