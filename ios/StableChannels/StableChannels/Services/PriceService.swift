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

        // Constants.defaultPriceFeeds is PriceOracle.directUSDFeeds in production and the
        // local E2E feed set when TestOverrides supplies one.
        let usdPrices = await Self.fetchFeeds(Constants.defaultPriceFeeds)
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
                PriceOracleAnchorStore.save(
                    price: result.price,
                    suiteName: Constants.appGroupIdentifier,
                    acceptedAt: self.lastUpdate
                )
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

    /// Delegated to PriceChartService for Single Responsibility separation.
    func fetchKrakenOHLC(since: Int64? = nil) async -> [(timestamp: Int64, price: Double)] {
        await PriceChartService.shared.fetchKrakenOHLC(since: since)
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
            guard let price = feed.extractPrice(from: jsonObject) else {
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
}
