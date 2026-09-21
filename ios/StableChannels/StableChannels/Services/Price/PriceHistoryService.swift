import Foundation

/// Service responsible for fetching, converting, caching, and downsampling price series.
/// Implements `PriceHistoryProviding` to keep UI components free of database orchestration.
actor PriceHistoryService: PriceHistoryProviding {
    private let databaseProvider: @Sendable () -> DatabaseService?
    private var allDailyPrices: [PriceRecord] = []
    private var hourlyPrices: [PriceRecord] = []
    private var isLoaded = false

    init(databaseService: DatabaseService?) {
        self.databaseProvider = { databaseService }
    }

    func fetchPriceHistory(for period: ChartPeriod, force: Bool = false) async -> [PriceRecord] {
        ensureLoaded(force: force)

        let cutoff = Date().addingTimeInterval(-Double(period.days) * 86400)
        let raw: [PriceRecord]

        if period.usesHourly {
            let startIdx = PriceChartAlgorithms.lowerBound(in: hourlyPrices, cutoff: cutoff)
            let hourlySlice = Array(hourlyPrices[startIdx...])
            if hourlySlice.count >= 2 {
                raw = hourlySlice
            } else {
                let dailyStartIdx = PriceChartAlgorithms.lowerBound(in: allDailyPrices, cutoff: cutoff)
                let dailySlice = Array(allDailyPrices[dailyStartIdx...])
                raw = dailySlice.count >= 2 ? dailySlice : hourlySlice
            }
        } else {
            let startIdx = PriceChartAlgorithms.lowerBound(in: allDailyPrices, cutoff: cutoff)
            let dailySlice = Array(allDailyPrices[startIdx...])
            if dailySlice.count >= 2 {
                raw = dailySlice
            } else {
                let hourlyStartIdx = PriceChartAlgorithms.lowerBound(in: hourlyPrices, cutoff: cutoff)
                let hourlySlice = Array(hourlyPrices[hourlyStartIdx...])
                raw = hourlySlice.count >= 2 ? hourlySlice : dailySlice
            }
        }

        return PriceChartAlgorithms.lttbDownsample(raw, targetCount: 200)
    }

    private func ensureLoaded(force: Bool) {
        if isLoaded && !force { return }
        guard let databaseService = databaseProvider() else { return }

        hourlyPrices = (try? databaseService.priceRepo.getPriceHistory(hours: 24 * 30)) ?? []

        let dailyRecords: [DailyPriceRecord] = (try? databaseService.priceRepo.getDailyPrices(days: 99999)) ?? []
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.timeZone = TimeZone(secondsFromGMT: 0)

        allDailyPrices = dailyRecords.compactMap { daily in
            guard let date = formatter.date(from: daily.date) else { return nil }
            let timestamp = Int64(date.timeIntervalSince1970)
            return PriceRecord(
                id: timestamp,
                price: daily.close,
                source: "daily",
                timestamp: timestamp
            )
        }

        if !hourlyPrices.isEmpty || !allDailyPrices.isEmpty {
            isLoaded = true
        }
    }
}
