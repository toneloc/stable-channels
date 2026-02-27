import SwiftUI
import Charts

struct HistoryView: View {
    @Environment(AppState.self) private var appState
    @State private var trades: [TradeRecord] = []
    @State private var payments: [PaymentRecord] = []
    @State private var priceHistory: [PriceRecord] = []
    @State private var selectedSegment = 0
    @State private var selectedTrade: TradeRecord?
    @State private var selectedPayment: PaymentRecord?
    @State private var chartPeriod: ChartPeriod = .day

    enum ChartPeriod: String, CaseIterable {
        case day = "1D"
        case week = "1W"
        case month = "1M"
        case year = "1Y"

        var hours: UInt32 {
            switch self {
            case .day: return 24
            case .week: return 168
            case .month: return 720
            case .year: return 8760
            }
        }
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Price chart
                if appState.btcPrice > 0 {
                    priceChartSection
                }

                // Segment picker
                Picker("History", selection: $selectedSegment) {
                    Text("Trades").tag(0)
                    Text("Payments").tag(1)
                }
                .pickerStyle(.segmented)
                .padding()

                // List
                List {
                    if selectedSegment == 0 {
                        tradesList
                    } else {
                        paymentsList
                    }
                }
                .listStyle(.plain)
                .overlay {
                    if selectedSegment == 0 && trades.isEmpty {
                        ContentUnavailableView("No Trades", systemImage: "arrow.left.arrow.right",
                            description: Text("Buy or sell BTC to see trades here."))
                    } else if selectedSegment == 1 && payments.isEmpty {
                        ContentUnavailableView("No Payments", systemImage: "bolt.fill",
                            description: Text("Send or receive payments to see history here."))
                    }
                }
            }
            .navigationTitle("History")
            .task {
                loadHistory()
            }
            .refreshable {
                loadHistory()
            }
            .sheet(item: $selectedTrade) { trade in
                TradeDetailView(trade: trade)
            }
            .sheet(item: $selectedPayment) { payment in
                PaymentDetailView(payment: payment)
            }
        }
    }

    // MARK: - Price Chart

    private var priceChartSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("BTC Price")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(appState.btcPrice.usdFormatted)
                        .font(.title3.bold())
                }

                Spacer()

                // Chart period selector
                Picker("Period", selection: $chartPeriod) {
                    ForEach(ChartPeriod.allCases, id: \.self) { period in
                        Text(period.rawValue).tag(period)
                    }
                }
                .pickerStyle(.segmented)
                .frame(width: 180)
            }
            .padding(.horizontal)

            if priceHistory.count >= 2 {
                Chart(priceHistory) { record in
                    LineMark(
                        x: .value("Time", record.date),
                        y: .value("Price", record.price)
                    )
                    .foregroundStyle(.blue)
                    .interpolationMethod(.catmullRom)
                }
                .chartYScale(domain: .automatic(includesZero: false))
                .chartXAxis(.hidden)
                .chartYAxis {
                    AxisMarks(position: .trailing) { value in
                        AxisValueLabel {
                            if let price = value.as(Double.self) {
                                Text(price.usdFormatted)
                                    .font(.caption2)
                            }
                        }
                    }
                }
                .frame(height: 120)
                .padding(.horizontal)
            } else {
                RoundedRectangle(cornerRadius: 8)
                    .fill(.quaternary)
                    .frame(height: 120)
                    .overlay {
                        Text("Collecting price data...")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                    .padding(.horizontal)
            }
        }
        .padding(.vertical, 8)
        .onChange(of: chartPeriod) {
            loadPriceHistory()
        }
    }

    // MARK: - Trades List

    private var tradesList: some View {
        ForEach(trades) { trade in
            Button { selectedTrade = trade } label: {
                TradeRowView(trade: trade)
            }
            .tint(.primary)
        }
    }

    // MARK: - Payments List

    private var paymentsList: some View {
        ForEach(payments) { payment in
            Button { selectedPayment = payment } label: {
                PaymentRowView(payment: payment)
            }
            .tint(.primary)
        }
    }

    private func loadHistory() {
        trades = (try? appState.databaseService?.getRecentTrades(limit: 50)) ?? []
        payments = (try? appState.databaseService?.getRecentPayments(limit: 50)) ?? []
        loadPriceHistory()
    }

    private func loadPriceHistory() {
        priceHistory = (try? appState.databaseService?.getPriceHistory(hours: chartPeriod.hours)) ?? []
    }
}

// MARK: - Row Views

struct TradeRowView: View {
    let trade: TradeRecord

    private var isBuy: Bool { trade.action == "buy" }

    var body: some View {
        HStack {
            Image(systemName: isBuy ? "arrow.up.right.circle.fill" : "arrow.down.right.circle.fill")
                .foregroundStyle(isBuy ? .orange : .purple)
                .font(.title3)

            VStack(alignment: .leading, spacing: 2) {
                Text(isBuy ? "Buy BTC" : "Sell BTC")
                    .fontWeight(.medium)
                Text(trade.date, style: .relative)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 2) {
                Text(String(format: "$%.2f", trade.amountUSD))
                    .fontWeight(.medium)
                Text(statusLabel)
                    .font(.caption)
                    .foregroundStyle(statusColor)
            }
        }
        .padding(.vertical, 4)
    }

    private var statusLabel: String { trade.status.capitalized }
    private var statusColor: Color {
        switch trade.status {
        case "completed": return .green
        case "pending": return .orange
        case "failed": return .red
        default: return .secondary
        }
    }
}

struct PaymentRowView: View {
    let payment: PaymentRecord

    var body: some View {
        HStack {
            Image(systemName: payment.isIncoming ? "arrow.down.circle.fill" : "arrow.up.circle.fill")
                .foregroundStyle(payment.isIncoming ? .green : .blue)
                .font(.title3)

            VStack(alignment: .leading, spacing: 2) {
                Text(payment.isIncoming ? "Received" : "Sent")
                    .fontWeight(.medium)
                Text(paymentTypeLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 2) {
                Text(payment.amountSats.satsFormatted)
                    .fontWeight(.medium)
                Text(statusLabel)
                    .font(.caption)
                    .foregroundStyle(statusColor)
            }
        }
        .padding(.vertical, 4)
    }

    private var paymentTypeLabel: String {
        switch payment.paymentType {
        case "stability": return "Stability"
        case "lightning": return "Lightning"
        case "splice_in": return "Splice In"
        case "splice_out": return "Splice Out"
        default: return payment.paymentType
        }
    }

    private var statusLabel: String { payment.status.capitalized }
    private var statusColor: Color {
        switch payment.status {
        case "completed": return .green
        case "pending": return .orange
        case "failed": return .red
        default: return .secondary
        }
    }
}
