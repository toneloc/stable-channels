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

    var compact: Bool = false

    private let selectionFeedback = UISelectionFeedbackGenerator()

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
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

            // Period selector buttons
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(ChartPeriod.allCases, id: \.self) { period in
                        let isSelected = chartPeriod == period
                        Button {
                            selectionFeedback.selectionChanged()
                            withAnimation(.spring(response: 0.28, dampingFraction: 0.8)) {
                                chartPeriod = period
                            }
                        } label: {
                            Text(period.rawValue)
                                .font(.system(size: 11, weight: isSelected ? .bold : .medium))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .background(
                                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                                        .fill(isSelected ? Color.blue : Color(.systemGray5))
                                )
                                .foregroundStyle(isSelected ? Color.white : Color.primary)
                        }
                        .buttonStyle(.plain)
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
                                    .font(.system(size: 10, weight: .medium))
                                    .foregroundStyle(Color.primary.opacity(0.65))
                            }
                        }
                    }
                }
                .chartYAxis {
                    AxisMarks(position: .trailing, values: .automatic(desiredCount: 4)) { value in
                        AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5, dash: [4, 4]))
                            .foregroundStyle(Color(.separator).opacity(0.5))
                        AxisValueLabel {
                            if let price = value.as(Double.self) {
                                Text(formatYAxis(price))
                                    .font(.system(size: 10, weight: .semibold, design: .rounded))
                                    .foregroundStyle(Color.primary.opacity(0.75))
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
                                        let record = PriceChartAlgorithms.nearestRecord(
                                            in: priceHistory,
                                            targetDate: date
                                        )
                                        if selectedPricePoint?.id != record?.id {
                                            selectedPricePoint = record
                                            selectionFeedback.selectionChanged()
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
            await loadHistory()
        }
        .onChange(of: chartPeriod) {
            selectedPricePoint = nil
            Task { await loadHistory() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .priceHistoryUpdated)) { _ in
            Task { await loadHistory(force: true) }
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

    private func loadHistory(force: Bool = false) async {
        let records = await appState.priceHistoryProvider.fetchPriceHistory(for: chartPeriod, force: force)
        await MainActor.run {
            self.priceHistory = records
        }
    }
}

// Equatable wrapper so HomeView re-renders (price ticks, sheet toggles, etc.)
// don't propagate into the chart at all. Only the chart's own state can trigger re-renders.
struct PriceChartCard: View, Equatable {
    let compact: Bool
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.compact == rhs.compact }
    var body: some View { PriceChartView(compact: compact) }
}

#Preview("Price Chart") {
    PriceChartCard(compact: false)
        .environment(AppState())
}
