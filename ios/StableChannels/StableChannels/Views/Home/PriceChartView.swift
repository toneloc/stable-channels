import SwiftUI
import Charts

struct PriceChartView: View {
    @Environment(AppState.self) private var appState
    @State private var priceHistory: [PriceRecord] = []
    @State private var chartPeriod: ChartPeriod = .all
    @State private var selectedPricePoint: PriceRecord?
    @State private var allDailyPrices: [PriceRecord] = []
    @State private var hourlyPrices: [PriceRecord] = []
    @State private var dataLoaded = false

    var compact: Bool = false

    enum ChartPeriod: String, CaseIterable {
        case day = "1D"
        case week = "1W"
        case month = "1M"
        case threeMonth = "3M"
        case sixMonth = "6M"
        case ytd = "YTD"
        case year = "1Y"
        case twoYear = "2Y"
        case fiveYear = "5Y"
        case tenYear = "10Y"
        case all = "ALL"

        var days: UInt32 {
            switch self {
            case .day: return 1
            case .week: return 7
            case .month: return 30
            case .threeMonth: return 90
            case .sixMonth: return 180
            case .ytd:
                let now = Date()
                let jan1 = Calendar.current.date(from: Calendar.current.dateComponents([.year], from: now))!
                return UInt32(now.timeIntervalSince(jan1) / 86400) + 1
            case .year: return 365
            case .twoYear: return 730
            case .fiveYear: return 1825
            case .tenYear: return 3650
            case .all: return 99999
            }
        }

        var usesHourly: Bool {
            switch self {
            case .day, .week, .month: return true
            default: return false
            }
        }

        var dateFormat: Date.FormatStyle {
            switch self {
            case .day:
                return .dateTime.hour().minute()
            case .week, .month, .threeMonth:
                return .dateTime.month(.abbreviated).day()
            case .sixMonth, .ytd, .year:
                return .dateTime.month(.abbreviated).year(.twoDigits)
            default:
                return .dateTime.month(.abbreviated).year()
            }
        }
    }

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
                            Text("BTC Price")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Text(appState.btcPrice.usdFormatted)
                            .font(.title3.bold())
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
                                Text(date, format: xAxisFormat)
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
                                        if let closest = priceHistory.min(by: {
                                            abs($0.date.timeIntervalSince(date)) < abs($1.date.timeIntervalSince(date))
                                        }) {
                                            selectedPricePoint = closest
                                        }
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
                    .fill(.quaternary)
                    .frame(height: compact ? 220 : 150)
                    .overlay {
                        Text("Collecting price data...")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
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
    }

    // MARK: - Axis Helpers

    private var chartMin: Double {
        let prices = priceHistory.map(\.price)
        let min = prices.min() ?? 0
        return min * 0.98 // 2% padding below
    }

    private var chartMax: Double {
        let prices = priceHistory.map(\.price)
        let max = prices.max() ?? 100
        return max * 1.02 // 2% padding above
    }

    private var xAxisFormat: Date.FormatStyle {
        switch chartPeriod {
        case .day:
            return .dateTime.hour()
        case .week, .month:
            return .dateTime.month(.abbreviated).day()
        case .threeMonth, .sixMonth, .ytd:
            return .dateTime.month(.abbreviated)
        default:
            return .dateTime.year()
        }
    }

    private func formatYAxis(_ price: Double) -> String {
        if price >= 1000 {
            return "$\(Int(price / 1000))K"
        } else {
            return "$\(Int(price))"
        }
    }

    // MARK: - Data Loading

    private func loadAllData() {
        guard !dataLoaded else { return }
        // Load all hourly data (up to 30 days)
        hourlyPrices = (try? appState.databaseService?.getPriceHistory(hours: 24 * 30)) ?? []

        // Load all daily data
        let dailyPrices = (try? appState.databaseService?.getDailyPrices(days: 99999)) ?? []
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        allDailyPrices = dailyPrices.compactMap { daily in
            guard let date = formatter.date(from: daily.date) else { return nil }
            return PriceRecord(
                id: Int64(date.timeIntervalSince1970),
                price: daily.close,
                source: "daily",
                timestamp: Int64(date.timeIntervalSince1970)
            )
        }
        dataLoaded = true
    }

    private func filterForPeriod() {
        let cutoff = Date().addingTimeInterval(-Double(chartPeriod.days) * 86400)

        if chartPeriod.usesHourly {
            priceHistory = hourlyPrices.filter { $0.date >= cutoff }
        } else {
            priceHistory = allDailyPrices.filter { $0.date >= cutoff }
        }
    }
}
