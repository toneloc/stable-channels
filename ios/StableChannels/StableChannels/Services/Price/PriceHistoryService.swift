import Foundation

/// Service responsible for fetching, converting, caching, and downsampling price series.
/// Implements `PriceHistoryProviding` to keep UI components free of database orchestration.
actor PriceHistoryService: PriceHistoryProviding {
    private let databaseProvider: @Sendable () -> DatabaseService?
    private var allDailyPrices: [PriceRecord] = []
    private var hourlyPrices: [PriceRecord] = []
    private var isLoaded = false

    init(databaseProvider: @escaping @Sendable () -> DatabaseService?) {
        self.databaseProvider = databaseProvider
    }

    init(databaseService: DatabaseService?) {
        self.databaseProvider = { databaseService }
    }

    func fetchPriceHistory(for period: ChartPeriod, force: Bool = false) async -> [PriceRecord] {
        ensureLoaded(force: force)

        let cutoff = Date().addingTimeInterval(-Double(period.days) * 86400)

        if period.usesHourly {
            let startIdx = PriceChartAlgorithms.lowerBound(in: hourlyPrices, cutoff: cutoff)
            let hourlySlice = hourlyPrices[startIdx...]
            if hourlySlice.count >= 2 {
                return PriceChartAlgorithms.lttbDownsample(hourlySlice, targetCount: 200)
            } else {
                let dailyStartIdx = PriceChartAlgorithms.lowerBound(in: allDailyPrices, cutoff: cutoff)
                let dailySlice = allDailyPrices[dailyStartIdx...]
                return PriceChartAlgorithms.lttbDownsample(
                    dailySlice.count >= 2 ? dailySlice : hourlySlice,
                    targetCount: 200
                )
            }
        } else {
            let startIdx = PriceChartAlgorithms.lowerBound(in: allDailyPrices, cutoff: cutoff)
            let dailySlice = allDailyPrices[startIdx...]
            if dailySlice.count >= 2 {
                return PriceChartAlgorithms.lttbDownsample(dailySlice, targetCount: 200)
            } else {
                let hourlyStartIdx = PriceChartAlgorithms.lowerBound(in: hourlyPrices, cutoff: cutoff)
                let hourlySlice = hourlyPrices[hourlyStartIdx...]
                return PriceChartAlgorithms.lttbDownsample(
                    hourlySlice.count >= 2 ? hourlySlice : dailySlice,
                    targetCount: 200
                )
            }
        }
    }

    private func ensureLoaded(force: Bool) {
        if isLoaded && !force { return }
        guard let databaseService = databaseProvider() else { return }

        hourlyPrices = (try? databaseService.priceRepo.getPriceHistory(hours: 24 * 30)) ?? []

        let dailyRecords: [DailyPriceRecord] = (try? databaseService.priceRepo.getDailyPrices(days: 99999)) ?? []

        allDailyPrices = dailyRecords.compactMap { daily in
            guard let timestamp = Self.parseDailyDateToTimestamp(daily.date) else { return nil }
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

    /// Lockless, zero-allocation ASCII parsing for "yyyy-MM-dd" UTC dates.
    /// Strictly rejects malformed or non-existent calendar dates (e.g. leap day checks, 30-day months).
    static func parseDailyDateToTimestamp(_ dateString: String) -> Int64? {
        guard dateString.utf8.count == 10 else { return nil }

        let utf8 = dateString.utf8
        var y: Int64 = 0
        var m: Int64 = 0
        var d: Int64 = 0
        var idx = utf8.startIndex

        for _ in 0..<4 {
            let byte = utf8[idx]
            guard byte >= 48, byte <= 57 else { return nil }
            y = y * 10 + Int64(byte - 48)
            idx = utf8.index(after: idx)
        }
        guard utf8[idx] == 45 else { return nil } // '-'
        idx = utf8.index(after: idx)

        for _ in 0..<2 {
            let byte = utf8[idx]
            guard byte >= 48, byte <= 57 else { return nil }
            m = m * 10 + Int64(byte - 48)
            idx = utf8.index(after: idx)
        }
        guard utf8[idx] == 45 else { return nil } // '-'
        idx = utf8.index(after: idx)

        for _ in 0..<2 {
            let byte = utf8[idx]
            guard byte >= 48, byte <= 57 else { return nil }
            d = d * 10 + Int64(byte - 48)
            idx = utf8.index(after: idx)
        }

        // Validate valid month and calendar days (including leap years)
        guard m >= 1, m <= 12, d >= 1, d <= daysInMonth(year: y, month: m) else { return nil }

        // Civil day computation from Gregorian (year, month, day)
        y -= (m <= 2 ? 1 : 0)
        let era = (y >= 0 ? y : y - 399) / 400
        let yoe = y - era * 400
        let doy = (153 * (m > 2 ? m - 3 : m + 9) + 2) / 5 + d - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        let days = era * 146097 + doe - 719468
        return days * 86400
    }

    private static func daysInMonth(year: Int64, month: Int64) -> Int64 {
        switch month {
        case 4, 6, 9, 11:
            return 30
        case 2:
            let isLeap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
            return isLeap ? 29 : 28
        default:
            return 31
        }
    }
}
