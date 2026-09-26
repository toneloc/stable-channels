import SwiftUI

struct PriceChartView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage("is_price_chart_expanded") private var isExpanded: Bool = true
    @State private var priceHistory: [PriceRecord] = []
    @State private var chartMin: Double = 0
    @State private var chartMax: Double = 100
    @State private var chartPeriod: ChartPeriod = .all
    @State private var displayedPeriod: ChartPeriod = .all
    @State private var selectedPricePoint: PriceRecord?
    @State private var loadTask: Task<Void, Never>?
    @State private var isHistoryDirty = false

    var compact: Bool = false

    private let impactFeedback = UIImpactFeedbackGenerator(style: .light)

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            headerButton

            if isExpanded {
                expandedContent
            }
        }
        .background(cardBackground)
        .overlay(cardBorder)
        .shadow(color: cardShadowColor, radius: 8, x: 0, y: 2)
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .onAppear {
            if isExpanded {
                loadHistory(for: chartPeriod, force: isHistoryDirty)
                isHistoryDirty = false
            }
        }
        .onChange(of: isExpanded) { _, newValue in
            if newValue {
                let force = isHistoryDirty
                isHistoryDirty = false
                loadHistory(for: chartPeriod, force: force)
            }
        }
        .onChange(of: chartPeriod) { _, newPeriod in
            var transaction = Transaction()
            transaction.disablesAnimations = true
            withTransaction(transaction) {
                selectedPricePoint = nil
            }
            let force = isHistoryDirty
            isHistoryDirty = false
            loadHistory(for: newPeriod, force: force)
        }
        .onReceive(NotificationCenter.default.publisher(for: .priceHistoryUpdated)) { _ in
            if isExpanded {
                loadHistory(for: chartPeriod, force: true)
            } else {
                isHistoryDirty = true
            }
        }
    }

    // MARK: - Header

    private var headerButton: some View {
        Button {
            impactFeedback.impactOccurred()
            withAnimation(.spring(response: 0.35, dampingFraction: 0.82)) {
                isExpanded.toggle()
            }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "bitcoinsign.circle.fill")
                    .font(.system(size: 22))
                    .foregroundStyle(.orange)

                VStack(alignment: .leading, spacing: 2) {
                    Text(String(localized: "label_btc_price", defaultValue: "BTC Price"))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                    if let selected = selectedPricePoint {
                        Text(selected.date, format: chartPeriod.dateFormat)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }

                Spacer()

                if let selected = selectedPricePoint {
                    Text(selected.price.usdFormatted)
                        .font(.headline.bold().monospacedDigit())
                        .foregroundStyle(.primary)
                } else {
                    LivePriceLabel()
                }

                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    // MARK: - Expanded Content

    private var expandedContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            PriceChartPeriodSelectorView(chartPeriod: $chartPeriod)

            PriceChartGraphView(
                priceHistory: priceHistory,
                chartMin: chartMin,
                chartMax: chartMax,
                chartPeriod: displayedPeriod,
                selectedPricePoint: $selectedPricePoint,
                compact: compact
            )
        }
        .padding(.top, 10)
        .padding(.bottom, 12)
    }

    // MARK: - Card Styling

    private var cardBackground: some View {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
            .fill(colorScheme == .dark ? Color(white: 0.11) : Color(white: 0.96))
    }

    private var cardBorder: some View {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
            .strokeBorder(
                colorScheme == .dark ? Color.white.opacity(0.08) : Color(.separator).opacity(0.30),
                lineWidth: 1
            )
    }

    private var cardShadowColor: Color {
        colorScheme == .dark ? Color.clear : Color.black.opacity(0.04)
    }

    // MARK: - Data Loading

    private func loadHistory(for period: ChartPeriod, force: Bool = false) {
        loadTask?.cancel()
        loadTask = Task {
            let records = await appState.priceHistoryProvider.fetchPriceHistory(for: period, force: force)
            guard !Task.isCancelled else { return }
            let bounds = PriceChartAlgorithms.chartBounds(in: records)
            await MainActor.run {
                guard !Task.isCancelled else { return }
                withAnimation(.easeInOut(duration: 0.24)) {
                    self.priceHistory = records
                    self.chartMin = bounds.min
                    self.chartMax = bounds.max
                    self.displayedPeriod = period
                }
            }
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
