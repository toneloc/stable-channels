import Foundation

@Observable
class PriceService {
    private(set) var currentPrice: Double = 0.0
    private(set) var lastUpdate: Date = .distantPast
    private(set) var isUpdating = false
    private var refreshTask: Task<Void, Never>?

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

    /// Fetch the median price from multiple feeds.
    func fetchPrice() async {
        guard !isUpdating else { return }
        await MainActor.run { isUpdating = true }

        let feeds = Constants.defaultPriceFeeds
        var prices: [Double] = []

        await withTaskGroup(of: (String, Double?).self) { group in
            for feed in feeds {
                group.addTask {
                    let price = await Self.fetchSingleFeed(feed)
                    return (feed.name, price)
                }
            }

            for await (_, price) in group {
                if let p = price, p > 0 {
                    prices.append(p)
                }
            }
        }

        let median = Self.median(prices)

        await MainActor.run {
            if median > 0 {
                self.currentPrice = median
                self.lastUpdate = Date()
            }
            self.isUpdating = false
        }
    }

    // MARK: - Private

    private static func fetchSingleFeed(_ feed: PriceFeedConfig) async -> Double? {
        let urlString = feed.urlFormat
            .replacingOccurrences(of: "{currency_lc}", with: "usd")
            .replacingOccurrences(of: "{currency}", with: "USD")

        guard let url = URL(string: urlString) else { return nil }

        for attempt in 0..<Constants.priceFetchMaxRetries {
            do {
                let (data, response) = try await URLSession.shared.data(from: url)
                guard let httpResponse = response as? HTTPURLResponse,
                      (200..<300).contains(httpResponse.statusCode) else {
                    continue
                }

                guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                    return nil
                }

                return extractPrice(from: json, path: feed.jsonPath)
            } catch {
                if attempt < Constants.priceFetchMaxRetries - 1 {
                    try? await Task.sleep(nanoseconds: Constants.priceFetchRetryDelayMs * 1_000_000)
                }
            }
        }
        return nil
    }

    private static func extractPrice(from json: [String: Any], path: [String]) -> Double? {
        var current: Any = json
        for key in path {
            if let dict = current as? [String: Any], let next = dict[key] {
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

    private static func median(_ values: [Double]) -> Double {
        guard !values.isEmpty else { return 0 }
        let sorted = values.sorted()
        let count = sorted.count
        if count % 2 == 0 {
            return (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
        } else {
            return sorted[count / 2]
        }
    }
}
