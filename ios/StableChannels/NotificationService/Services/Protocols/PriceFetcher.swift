import Foundation

/// Protocol for fetching BTC/USD price
protocol PriceFetcher {
    func fetchPrice() -> Double
}

/// Concrete implementation fetching from multiple sources
struct ConcurrentPriceFetcher: PriceFetcher {
    private static let sources: [(String, (Data) -> Double?)] = [
        ("https://www.bitstamp.net/api/v2/ticker/btcusd/", { data in
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let s = json["last"] as? String, let p = Double(s) {
                return p
            }
            return nil
        }),
        ("https://api.coinbase.com/v2/prices/spot?currency=USD", { data in
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let d = json["data"] as? [String: Any],
               let s = d["amount"] as? String, let p = Double(s) {
                return p
            }
            return nil
        }),
        ("https://blockchain.info/ticker", { data in
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let usd = json["USD"] as? [String: Any],
               let p = usd["last"] as? Double {
                return p
            }
            return nil
        }),
        ("https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD", { data in
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let result = json["result"] as? [String: Any],
               let pair = result["XXBTZUSD"] as? [String: Any],
               let c = pair["c"] as? [Any],
               let s = c.first as? String, let p = Double(s) {
                return p
            }
            return nil
        }),
        ("https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd", { data in
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let btc = json["bitcoin"] as? [String: Any],
               let p = btc["usd"] as? Double {
                return p
            }
            return nil
        })
    ]

    func fetchPrice() -> Double {
        let lock = NSLock()
        var prices: [Double] = []
        let group = DispatchGroup()

        func append(_ p: Double) {
            lock.lock()
            prices.append(p)
            lock.unlock()
        }

        // All requests fire concurrently
        for (url, parser) in Self.sources {
            group.enter()
            guard let url = URL(string: url) else {
                group.leave()
                continue
            }
            URLSession.shared.dataTask(with: url) { data, _, _ in
                defer { group.leave() }
                if let data, let price = parser(data) {
                    append(price)
                }
            }.resume()
        }

        _ = group.wait(timeout: .now() + 8)

        guard !prices.isEmpty else { return 0 }
        let sorted = prices.sorted()
        return sorted[sorted.count / 2] // median
    }
}
