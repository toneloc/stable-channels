import Charts
import SwiftUI

struct PriceChartGraphView: View {
    let priceHistory: [PriceRecord]
    let chartMin: Double
    let chartMax: Double
    let chartPeriod: ChartPeriod
    @Binding var selectedPricePoint: PriceRecord?
    let compact: Bool

    private let selectionFeedback = UISelectionFeedbackGenerator()

    var body: some View {
        if priceHistory.count >= 2 {
            chartContent
        } else {
            placeholderState
        }
    }

    private var chartContent: some View {
        Chart(priceHistory) { record in
            AreaMark(
                x: .value("Time", record.date),
                yStart: .value("Min", chartMin),
                yEnd: .value("Price", record.price)
            )
            .foregroundStyle(
                LinearGradient(
                    colors: [.blue.opacity(0.18), .blue.opacity(0.02)],
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

            if let selected = selectedPricePoint, selected.id == record.id {
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
        .chartYScale(domain: chartMin ... chartMax)
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
        .frame(height: compact ? 200 : 150)
        .padding(.horizontal, 14)
    }

    private var placeholderState: some View {
        RoundedRectangle(cornerRadius: 10)
            .fill(Color(.tertiarySystemFill))
            .frame(height: compact ? 200 : 150)
            .overlay {
                VStack(spacing: 12) {
                    CurveProgressIndicator(curve: .spiralSearch, size: 68, tint: .blue)
                    Text(String(
                        localized: "status_collecting_data",
                        defaultValue: "Collecting price data..."
                    ))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 14)
    }

    private func formatYAxis(_ price: Double) -> String {
        if price >= 1000 {
            return "$\(Int(price / 1000))K"
        } else {
            return "$\(Int(price))"
        }
    }
}
