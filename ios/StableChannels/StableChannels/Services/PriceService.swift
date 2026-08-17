import Foundation

@Observable
class PriceService {
    var currentPrice: Double = 0.0
    private(set) var lastUpdate: Date = .distantPast
    private(set) var isUpdating = false
    private(set) var isTrustedForAccounting = false
    private(set) var isQuarantined = false
    private(set) var activeSource: PriceOracleSource?
    private var refreshTask: Task<Void, Never>?

    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = Constants.priceFetchTimeoutSecs
        configuration.timeoutIntervalForResource = Constants.priceFetchTimeoutSecs
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.waitsForConnectivity = false
        return URLSession(configuration: configuration)
    }()

    // MARK: - Public

    /// Start auto-refreshing prices every N seconds.
    func startAutoRefresh(intervalSecs: UInt64 = Constants.priceCacheRefreshSecs) {
        refreshTask?.cancel()
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.fetchPrice()
                try? await Task.sleep(nanoseconds: intervalSecs * 1_000_000_000)
            }
        }
    }

    func stopAutoRefresh() {
        refreshTask?.cancel()
        refreshTask = nil
    }

    /// The last accepted value remains available for display, but accounting fails closed once
    /// it is stale or quarantined by the large-move circuit breaker.
    var accountingPrice: Double {
        guard isTrustedForAccounting,
              !isQuarantined,
              Date().timeIntervalSince(lastUpdate) <= PriceOracle.maximumTrustedPriceAge else {
            return 0
        }
        return currentPrice
    }

    var isPriceStale: Bool {
        Date().timeIntervalSince(lastUpdate) > PriceOracle.maximumTrustedPriceAge
    }

    func seedDisplayPrice(_ price: Double) {
        guard currentPrice <= 0, PriceOracle.isPlausibleBitcoinPrice(price) else { return }
        currentPrice = price
        isTrustedForAccounting = false
    }

    /// Fetch a direct-USD consensus, falling back to peg-normalized USDT only when USD quorum fails.
    func fetchPrice() async {
        guard !isUpdating else { return }
        await MainActor.run { isUpdating = true }

        let lastTrustedPrice = await MainActor.run { () -> Double? in
            guard !self.isPriceStale,
                  PriceOracle.isPlausibleBitcoinPrice(self.currentPrice) else { return nil }
            return self.currentPrice
        }

        let usdPrices = await Self.fetchFeeds(PriceOracle.directUSDFeeds)
        do {
            let result: PriceOracleResult
            do {
                result = try PriceOracle.resolve(
                    usdPrices: usdPrices,
                    usdtPrices: [],
                    pegPrices: [],
                    lastTrustedPrice: lastTrustedPrice
                )
            } catch let failure as PriceOracleFailure where !failure.quarantinesPrice {
                print("[PriceOracle] direct USD unavailable: \(failure); trying USDT fallback")
                async let usdtPrices = Self.fetchFeeds(PriceOracle.bitcoinUSDTFeeds)
                async let pegPrices = Self.fetchFeeds(PriceOracle.usdtUSDFeeds)
                let (fetchedUSDTPrices, fetchedPegPrices) = await (usdtPrices, pegPrices)
                result = try PriceOracle.resolve(
                    usdPrices: [],
                    usdtPrices: fetchedUSDTPrices,
                    pegPrices: fetchedPegPrices,
                    lastTrustedPrice: lastTrustedPrice
                )
            }

            await MainActor.run {
                self.currentPrice = result.price
                self.lastUpdate = Date()
                self.isQuarantined = false
                self.isTrustedForAccounting = true
                self.activeSource = result.source
                self.isUpdating = false
            }
            let pegDetail = result.usdtUSD.map { String(format: ", USDT/USD=%.6f", $0) } ?? ""
            print(
                "[PriceOracle] accepted \(result.source.rawValue) price from " +
                    "\(result.agreeingFeedNames.count) feeds\(pegDetail)"
            )
        } catch {
            let quarantines = (error as? PriceOracleFailure)?.quarantinesPrice == true
            await MainActor.run {
                if quarantines {
                    self.isQuarantined = true
                }
                self.isTrustedForAccounting = !self.isQuarantined && !self.isPriceStale
                self.isUpdating = false
            }
            print("[PriceOracle] rejected refresh: \(error)")
        }
    }

    // MARK: - Kraken OHLC Backfill

    /// Fetch hourly OHLC candles from Kraken for the last ~30 days.
    /// Returns array of (unix_timestamp, close_price).
    func fetchKrakenOHLC(since: Int64? = nil) async -> [(timestamp: Int64, price: Double)] {
        let sinceTs = since ?? (Int64(Date().timeIntervalSince1970) - 30 * 24 * 3600)
        guard let url = URL(string: "https://api.kraken.com/0/public/OHLC?pair=XXBTZUSD&interval=60&since=\(sinceTs)")
        else {
            return []
        }

        do {
            let (data, response) = try await Self.session.data(from: url)
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

    // MARK: - Private

    private static func fetchFeeds(_ feeds: [PriceFeedConfig]) async -> [NamedPrice] {
        await withTaskGroup(of: NamedPrice?.self, returning: [NamedPrice].self) { group in
            for feed in feeds {
                group.addTask {
                    guard let price = await fetchSingleFeed(feed) else { return nil }
                    return NamedPrice(feedName: feed.name, value: price)
                }
            }

            var prices: [NamedPrice] = []
            for await result in group {
                if let result { prices.append(result) }
            }
            return prices
        }
    }

    private static func fetchSingleFeed(_ feed: PriceFeedConfig) async -> Double? {
        let urlString = feed.urlFormat
            .replacingOccurrences(of: "{currency_lc}", with: "usd")
            .replacingOccurrences(of: "{currency}", with: "USD")
        guard let url = URL(string: urlString) else { return nil }

        do {
            let (data, response) = try await session.data(from: url)
            guard let httpResponse = response as? HTTPURLResponse,
                  (200..<300).contains(httpResponse.statusCode) else {
                print("[PriceOracle] \(feed.name) failed: non-2xx response")
                return nil
            }
            let jsonObject = try JSONSerialization.jsonObject(with: data)
            guard let price = extractPrice(from: jsonObject, path: feed.jsonPath) else {
                print("[PriceOracle] \(feed.name) failed: invalid response path")
                return nil
            }
            print("[PriceOracle] \(feed.name) succeeded")
            return price
        } catch {
            print("[PriceOracle] \(feed.name) failed: \(error.localizedDescription)")
            return nil
        }
    }

    static func extractPrice(from json: Any, path: [String]) -> Double? {
        var current: Any = json
        for key in path {
            // Handle numeric keys (array indices)
            if let index = Int(key), let array = current as? [Any], array.indices.contains(index) {
                current = array[index]
            } else if let dict = current as? [String: Any], let next = dict[key] {
                current = next
            } else {
                return nil
            }
        }

        // Handle array values (e.g. Kraken's "c": ["<last>", "<vol>"])
        if let array = current as? [Any], let first = array.first {
            current = first
        }

        if let price = current as? Double {
            return price
        } else if let price = current as? Int {
            return Double(price)
        } else if let priceStr = current as? String, let price = Double(priceStr) {
            return price
        }

        return nil
    }

    static func median(_ values: [Double]) -> Double {
        PriceOracle.median(values) ?? 0
    }
}
