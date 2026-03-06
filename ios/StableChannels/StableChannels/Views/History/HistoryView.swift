import SwiftUI

struct HistoryView: View {
    @Environment(AppState.self) private var appState
    @State private var trades: [TradeRecord] = []
    @State private var payments: [PaymentRecord] = []
    @State private var selectedSegment = 0
    @State private var selectedTrade: TradeRecord?
    @State private var selectedPayment: PaymentRecord?

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
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
            .navigationBarTitleDisplayMode(.inline)
            .task {
                loadHistory()
            }
            .refreshable {
                appState.refreshBalances()
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
        case "stability": return "Settlement"
        case "lightning": return "Lightning"
        case "splice_in": return "Splice In"
        case "splice_out": return "Splice Out"
        case "onchain": return "On-chain"
        case "bolt12": return "Bolt12"
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
