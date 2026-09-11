import SwiftUI
import Charts

// Separate view so only the price label re-renders on each price tick,
// not the entire chart body with 700+ data points.
private struct LivePriceLabel: View {
    @Environment(AppState.self) private var appState
    var body: some View {
        Text(appState.btcPrice.usdFormatted)
            .font(.title3.bold())
    }
}

struct PriceChartView: View {
    @Environment(AppState.self) private var appState
    @State private var priceHistory: [PriceRecord] = []
    @State private var chartPeriod: ChartPeriod = .all
    @State private var selectedPricePoint: PriceRecord?
    @State private var allDailyPrices: [PriceRecord] = []
    @State private var hourlyPrices: [PriceRecord] = []
    @State private var dataLoaded = false

    var compact: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            // Price header
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 1) {
                    if let selected = selectedPricePoint {
                        Text(selected.date, format: chartPeriod.dateFormat)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        Text(selected.price.usdFormatted)
                            .font(.title3.bold())
                    } else {
                        if !compact {
                            Text(String(localized: "label_btc_price", defaultValue: "BTC Price"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        LivePriceLabel()
                    }
                }
                Spacer()
            }
            .padding(.horizontal)

            // Period selector pills — scrollable
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(ChartPeriod.allCases, id: \.self) { period in
                        Button {
                            chartPeriod = period
                        } label: {
                            Text(period.rawValue)
                                .font(.caption2.bold())
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .background(chartPeriod == period ? Color.blue : Color(.systemGray5))
                                .foregroundStyle(chartPeriod == period ? .white : .primary)
                                .clipShape(Capsule())
                        }
                    }
                }
                .padding(.horizontal)
            }

            if priceHistory.count >= 2 {
                Chart(priceHistory) { record in
                    AreaMark(
                        x: .value("Time", record.date),
                        yStart: .value("Min", chartMin),
                        yEnd: .value("Price", record.price)
                    )
                    .foregroundStyle(
                        LinearGradient(
                            colors: [.blue.opacity(0.15), .blue.opacity(0.02)],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    )

                    LineMark(
                        x: .value("Time", record.date),
                        y: .value("Price", record.price)
                    )
                    .foregroundStyle(.blue)
                    .lineStyle(StrokeStyle(lineWidth: selectedPricePoint != nil ? 1.5 : 2))
                    .interpolationMethod(.catmullRom)

                    if let selected = selectedPricePoint,
                       selected.id == record.id {
                        RuleMark(x: .value("Selected", selected.date))
                            .foregroundStyle(.gray.opacity(0.5))
                            .lineStyle(StrokeStyle(lineWidth: 1, dash: [4, 3]))
                        PointMark(
                            x: .value("Time", selected.date),
                            y: .value("Price", selected.price)
                        )
                        .foregroundStyle(.blue)
                        .symbolSize(40)
                    }
                }
                .chartYScale(domain: chartMin...chartMax)
                .chartXAxis {
                    AxisMarks(values: .automatic(desiredCount: 4)) { value in
                        AxisValueLabel {
                            if let date = value.as(Date.self) {
                                Text(date, format: chartPeriod.xAxisFormat)
                                    .font(.system(size: 9))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .chartYAxis {
                    AxisMarks(position: .trailing, values: .automatic(desiredCount: 4)) { value in
                        AxisGridLine(stroke: StrokeStyle(lineWidth: 0.3, dash: [4, 4]))
                            .foregroundStyle(.secondary.opacity(0.3))
                        AxisValueLabel {
                            if let price = value.as(Double.self) {
                                Text(formatYAxis(price))
                                    .font(.system(size: 9))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .chartOverlay { proxy in
                    GeometryReader { geometry in
                        Rectangle()
                            .fill(.clear)
                            .contentShape(Rectangle())
                            .gesture(
                                DragGesture(minimumDistance: 0)
                                    .onChanged { value in
                                        let x = value.location.x - geometry[proxy.plotAreaFrame].origin.x
                                        guard let date: Date = proxy.value(atX: x) else { return }
                                        selectedPricePoint = PriceChartAlgorithms.nearestRecord(
                                            in: priceHistory,
                                            targetDate: date
                                        )
                                    }
                                    .onEnded { _ in
                                        selectedPricePoint = nil
                                    }
                            )
                    }
                }
                .frame(height: compact ? 220 : 150)
                .padding(.horizontal)
            } else {
                RoundedRectangle(cornerRadius: 8)
                    .fill(.quaternary.opacity(0.6))
                    .frame(height: compact ? 220 : 150)
                    .overlay {
                        VStack(spacing: 12) {
                            CurveProgressIndicator(curve: .spiralSearch, size: 68, tint: .blue)
                            Text(String(localized: "status_collecting_data", defaultValue: "Collecting price data..."))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.horizontal)
            }
        }
        .padding(.vertical, 8)
        .task {
            loadAllData()
            filterForPeriod()
        }
        .onChange(of: chartPeriod) {
            selectedPricePoint = nil
            filterForPeriod()
        }
        .onReceive(NotificationCenter.default.publisher(for: .priceHistoryUpdated)) { _ in
            loadAllData(force: true)
            filterForPeriod()
        }
    }

    // MARK: - Axis Helpers

    private var chartBounds: (min: Double, max: Double) {
        PriceChartAlgorithms.chartBounds(in: priceHistory)
    }

    private var chartMin: Double { chartBounds.min }
    private var chartMax: Double { chartBounds.max }

    private func formatYAxis(_ price: Double) -> String {
        if price >= 1000 {
            return "$\(Int(price / 1000))K"
        } else {
            return "$\(Int(price))"
        }
    }

    // MARK: - Data Loading

    private func loadAllData(force: Bool = false) {
        if dataLoaded && !force { return }
        // Load all hourly data (up to 30 days)
        hourlyPrices = (try? appState.databaseService?.priceRepo.getPriceHistory(hours: 24 * 30)) ?? []

        // Load all daily data
        let dailyPrices: [DailyPriceRecord] = (try? appState.databaseService?.priceRepo.getDailyPrices(days: 99999)) ??
            []
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        allDailyPrices = dailyPrices.compactMap { daily in
            guard let date = formatter.date(from: daily.date) else { return nil }
            return PriceRecord(
                id: Int64(date.timeIntervalSince1970),
                price: daily.close,
                source: "daily",
                timestamp: Int64(date.timeIntervalSince1970)
            )
        }
        if !hourlyPrices.isEmpty || !allDailyPrices.isEmpty {
            dataLoaded = true
        }
    }

    private func filterForPeriod() {
        let cutoff = Date().addingTimeInterval(-Double(chartPeriod.days) * 86400)
        let raw: [PriceRecord]

        if chartPeriod.usesHourly {
            let startIdx = PriceChartAlgorithms.lowerBound(in: hourlyPrices, cutoff: cutoff)
            let hourlySlice = Array(hourlyPrices[startIdx...])
            if hourlySlice.count >= 2 {
                raw = hourlySlice
            } else {
                // Fallback to daily if hourly is still backfilling or empty
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
                // Fallback to hourly if daily is sparse or still backfilling
                let hourlyStartIdx = PriceChartAlgorithms.lowerBound(in: hourlyPrices, cutoff: cutoff)
                let hourlySlice = Array(hourlyPrices[hourlyStartIdx...])
                raw = hourlySlice.count >= 2 ? hourlySlice : dailySlice
            }
        }

        priceHistory = PriceChartAlgorithms.lttbDownsample(raw, targetCount: 120)
    }
}

// Equatable wrapper so HomeView re-renders (price ticks, sheet toggles, etc.)
// don't propagate into the chart at all. Only the chart's own state can trigger re-renders.
struct PriceChartCard: View, Equatable {
    let compact: Bool
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.compact == rhs.compact }
    var body: some View { PriceChartView(compact: compact) }
}

#Preview("Collecting Price Data") {
    PriceChartCard(compact: false)
        .environment(AppState())
}
