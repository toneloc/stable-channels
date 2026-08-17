import Foundation

/// Protocol for fetching BTC/USD price
protocol PriceFetcher {
    func fetchPrice() -> Double
}

/// Concrete implementation fetching from multiple sources
struct ConcurrentPriceFetcher: PriceFetcher {
    func fetchPrice() -> Double {
        let usdPrices = Self.fetchFeeds(PriceOracle.directUSDFeeds)
        do {
            return try PriceOracle.resolve(
                usdPrices: usdPrices,
                usdtPrices: [],
                pegPrices: [],
                lastTrustedPrice: nil
            ).price
        } catch let failure as PriceOracleFailure where !failure.quarantinesPrice {
            print("[PriceOracle:NSE] direct USD unavailable: \(failure); trying USDT fallback")
            let fallbackPrices = Self.fetchFeeds(PriceOracle.bitcoinUSDTFeeds + PriceOracle.usdtUSDFeeds)
            let usdtNames = Set(PriceOracle.bitcoinUSDTFeeds.map(\.name))
            let pegNames = Set(PriceOracle.usdtUSDFeeds.map(\.name))
            do {
                return try PriceOracle.resolve(
                    usdPrices: [],
                    usdtPrices: fallbackPrices.filter { usdtNames.contains($0.feedName) },
                    pegPrices: fallbackPrices.filter { pegNames.contains($0.feedName) },
                    lastTrustedPrice: nil
                ).price
            } catch {
                print("[PriceOracle:NSE] rejected fallback: \(error)")
                return 0
            }
        } catch {
            print("[PriceOracle:NSE] rejected refresh: \(error)")
            return 0
        }
    }

    private static func fetchFeeds(_ feeds: [PriceFeedConfig]) -> [NamedPrice] {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 3
        configuration.timeoutIntervalForResource = 3
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.waitsForConnectivity = false
        let session = URLSession(configuration: configuration)

        let lock = NSLock()
        var prices: [NamedPrice] = []
        let group = DispatchGroup()

        for feed in feeds {
            group.enter()
            let urlString = feed.urlFormat
                .replacingOccurrences(of: "{currency_lc}", with: "usd")
                .replacingOccurrences(of: "{currency}", with: "USD")
            guard let url = URL(string: urlString) else {
                group.leave()
                continue
            }
            session.dataTask(with: url) { data, response, error in
                defer { group.leave() }
                guard error == nil,
                      let http = response as? HTTPURLResponse,
                      (200 ..< 300).contains(http.statusCode),
                      let data,
                      let json = try? JSONSerialization.jsonObject(with: data),
                      let value = extractPrice(from: json, path: feed.jsonPath) else {
                    print("[PriceOracle:NSE] \(feed.name) failed")
                    return
                }
                lock.lock()
                prices.append(NamedPrice(feedName: feed.name, value: value))
                lock.unlock()
                print("[PriceOracle:NSE] \(feed.name) succeeded")
            }.resume()
        }

        _ = group.wait(timeout: .now() + 4)
        session.invalidateAndCancel()
        lock.lock()
        let snapshot = prices
        lock.unlock()
        return snapshot
    }

    private static func extractPrice(from json: Any, path: [String]) -> Double? {
        var current: Any = json
        for key in path {
            if let index = Int(key), let array = current as? [Any], array.indices.contains(index) {
                current = array[index]
            } else if let dictionary = current as? [String: Any], let next = dictionary[key] {
                current = next
            } else {
                return nil
            }
        }

        if let array = current as? [Any], let first = array.first {
            current = first
        }

        if let value = current as? Double { return value }
        if let value = current as? Int { return Double(value) }
        if let value = current as? String { return Double(value) }
        return nil
    }
}
