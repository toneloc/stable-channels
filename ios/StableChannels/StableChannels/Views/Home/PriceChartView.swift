import SwiftUI
import Charts

struct PriceChartView: View {
    @Environment(AppState.self) private var appState
    @State private var priceHistory: [PriceRecord] = []
    @State private var chartPeriod: ChartPeriod = .all
    @State private var selectedPricePoint: PriceRecord?

    var compact: Bool = false

    enum ChartPeriod: String, CaseIterable {
        case day = "1D"
        case week = "1W"
        case month = "1M"
        case year = "1Y"
        case threeYear = "3Y"
        case all = "ALL"

        var days: UInt32 {
            switch self {
            case .day: return 1
            case .week: return 7
            case .month: return 30
            case .year: return 365
            case .threeYear: return 1095
            case .all: return 99999
            }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            // Price header
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 1) {
                    if let selected = selectedPricePoint {
                        Text(selected.date, format: chartPeriod == .day
                            ? .dateTime.hour().minute()
                            : .dateTime.month().day().year())
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

            // Period selector pills
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(ChartPeriod.allCases, id: \.self) { period in
                        Button {
                            chartPeriod = period
                        } label: {
                            Text(period.rawValue)
                                .font(.caption2.bold())
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
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
                    LineMark(
                        x: .value("Time", record.date),
                        y: .value("Price", record.price)
                    )
                    .foregroundStyle(.blue)
                    .lineStyle(StrokeStyle(lineWidth: selectedPricePoint != nil ? 1.5 : 2.5))
                    .interpolationMethod(selectedPricePoint != nil ? .linear : .catmullRom)

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
                .chartYScale(domain: .automatic(includesZero: false))
                .chartXAxis(.hidden)
                .chartYAxis(.hidden)
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
        .task { loadPriceHistory() }
        .onChange(of: chartPeriod) {
            selectedPricePoint = nil
            loadPriceHistory()
        }
    }

    private func loadPriceHistory() {
        switch chartPeriod {
        case .day:
            priceHistory = (try? appState.databaseService?.getPriceHistory(hours: 24)) ?? []
        case .week:
            priceHistory = (try? appState.databaseService?.getPriceHistory(hours: 24 * 7)) ?? []
        case .month:
            priceHistory = (try? appState.databaseService?.getPriceHistory(hours: 24 * 30)) ?? []
        default:
            // 1Y, 3Y, ALL — daily granularity is fine at this scale
            let dailyPrices = (try? appState.databaseService?.getDailyPrices(days: chartPeriod.days)) ?? []
            let formatter = DateFormatter()
            formatter.dateFormat = "yyyy-MM-dd"
            priceHistory = dailyPrices.compactMap { daily in
                guard let date = formatter.date(from: daily.date) else { return nil }
                return PriceRecord(
                    id: Int64(date.timeIntervalSince1970),
                    price: daily.close,
                    source: "daily",
                    timestamp: Int64(date.timeIntervalSince1970)
                )
            }
        }
    }
}
