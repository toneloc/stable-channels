import Foundation

/// Protocol for fetching BTC/USD price
protocol PriceFetcher {
    func fetchPrice() -> Double
}

/// Concrete implementation fetching from multiple sources
struct ConcurrentPriceFetcher: PriceFetcher {
    func fetchPrice() -> Double {
        let lastTrustedPrice = PriceOracleAnchorStore.freshPrice(suiteName: Constants.appGroup)
        let usdPrices = Self.fetchFeeds(PriceOracle.directUSDFeeds)
        do {
            let result = try PriceOracle.resolve(
                usdPrices: usdPrices,
                usdtPrices: [],
                pegPrices: [],
                lastTrustedPrice: lastTrustedPrice
            )
            return accept(result)
        } catch let failure as PriceOracleFailure where !failure.quarantinesPrice {
            print("[PriceOracle:NSE] direct USD unavailable: \(failure); trying USDT fallback")
            let fallbackPrices = Self.fetchFeeds(PriceOracle.bitcoinUSDTFeeds + PriceOracle.usdtUSDFeeds)
            let usdtNames = Set(PriceOracle.bitcoinUSDTFeeds.map(\.name))
            let pegNames = Set(PriceOracle.usdtUSDFeeds.map(\.name))
            do {
                let result = try PriceOracle.resolve(
                    usdPrices: [],
                    usdtPrices: fallbackPrices.filter { usdtNames.contains($0.feedName) },
                    pegPrices: fallbackPrices.filter { pegNames.contains($0.feedName) },
                    lastTrustedPrice: lastTrustedPrice
                )
                return accept(result)
            } catch {
                print("[PriceOracle:NSE] rejected fallback: \(error)")
                return 0
            }
        } catch {
            print("[PriceOracle:NSE] rejected refresh: \(error)")
            return 0
        }
    }

    private func accept(_ result: PriceOracleResult) -> Double {
        PriceOracleAnchorStore.save(price: result.price, suiteName: Constants.appGroup)
        return result.price
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
                      let value = feed.extractPrice(from: json) else {
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
}
