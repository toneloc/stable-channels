import SwiftUI

struct HistoryView: View {
    @Environment(AppState.self) private var appState
    @State private var trades: [TradeRecord] = []
    @State private var payments: [PaymentRecord] = []
    @State private var selectedSegment = 0
    @State private var selectedTrade: TradeRecord?
    @Environment(PaymentDetailCoordinator.self) private var paymentCoordinator

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Segment picker
                Picker(String(localized: "picker_history", defaultValue: "History"), selection: $selectedSegment) {
                    Text(String(localized: "segment_trades", defaultValue: "Orders")).tag(0)
                    Text(String(localized: "segment_payments", defaultValue: "Payments")).tag(1)
                }
                .pickerStyle(.segmented)
                .padding()
                .onChange(of: paymentCoordinator.selectedPayment) { _, newValue in
                    if let payment = newValue {
                        selectedSegment = payment.paymentType == "trade" ? 0 : 1
                    }
                }

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
                        ContentUnavailableView(
                            String(localized: "empty_trades_title", defaultValue: "No Trades"),
                            systemImage: "arrow.left.arrow.right",
                            description: Text(String(
                                localized: "empty_trades_desc",
                                defaultValue: "Convert BTC to see orders here."
                            ))
                        )
                    } else if selectedSegment == 1 && payments.isEmpty {
                        ContentUnavailableView(
                            String(localized: "empty_payments_title", defaultValue: "No Payments"),
                            systemImage: "bolt.fill",
                            description: Text(String(
                                localized: "empty_payments_desc",
                                defaultValue: "Send or receive payments to see history here."
                            ))
                        )
                    }
                }
            }
            .navigationTitle(String(localized: "title_history", defaultValue: "History"))
            .navigationBarTitleDisplayMode(.inline)
            .onAppear {
                loadHistory()
            }
            .refreshable {
                appState.refreshBalances()
                loadHistory()
            }
            .sheet(item: $selectedTrade) { trade in
                TradeDetailView(trade: trade)
            }
        }
    }

    private var historyDisplayPrice: Double {
        appState.btcPrice > 0 ? appState.btcPrice : appState.stableChannel.latestPrice
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
            Button { paymentCoordinator.open(payment) } label: {
                PaymentRowView(payment: payment, displayPrice: historyDisplayPrice)
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
                Text(isBuy
                    ? String(localized: "trade_buy_btc", defaultValue: "USD → BTC")
                    : String(localized: "trade_sell_btc", defaultValue: "BTC → USD"))
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
    let displayPrice: Double

    var body: some View {
        HStack {
            Image(systemName: payment.isIncoming ? "arrow.down.circle.fill" : "arrow.up.circle.fill")
                .foregroundStyle(payment.isIncoming ? .green : .blue)
                .font(.title3)

            VStack(alignment: .leading, spacing: 2) {
                Text(payment.isIncoming
                    ? String(localized: "payment_received", defaultValue: "Received")
                    : String(localized: "payment_sent", defaultValue: "Sent"))
                    .fontWeight(.medium)
                Text(paymentTypeLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 2) {
                if let usd = payment.displayUSD(fallbackPrice: displayPrice) {
                    Text(usd.usdFormatted)
                        .fontWeight(.medium)
                } else {
                    Text(payment.amountSats.satsFormatted)
                        .fontWeight(.medium)
                }
                Text(statusLabel)
                    .font(.caption)
                    .foregroundStyle(statusColor)
            }
        }
        .padding(.vertical, 4)
    }

    private var paymentTypeLabel: String {
        switch payment.paymentType {
        case "stability": return String(localized: "payment_type_stability", defaultValue: "Stability")
        case "lightning": return String(localized: "payment_type_lightning", defaultValue: "Lightning")
        case "splice_in": return String(localized: "payment_type_splice_in", defaultValue: "Splice In")
        case "splice_out": return String(localized: "payment_type_splice_out", defaultValue: "Splice Out")
        case "onchain": return String(localized: "payment_type_on_chain", defaultValue: "Onchain")
        case "channel_close": return String(localized: "payment_type_channel_close", defaultValue: "Channel Close")
        case "bolt12": return String(localized: "payment_type_bolt12", defaultValue: "Bolt12")
        default: return payment.paymentType
        }
    }

    private var statusLabel: String {
        if payment.shouldShowConfirmationProgress,
           !payment.confirmationStatusLabel.isEmpty {
            return payment.confirmationStatusLabel
        }
        return payment.status.capitalized
    }

    private var statusColor: Color {
        if payment.shouldShowConfirmationProgress,
           !payment.confirmationStatusLabel.isEmpty {
            return payment.confirmationStatusLabel == "Confirmed" ? .green : .orange
        }
        switch payment.status {
        case "completed": return .green
        case "pending": return .orange
        case "failed": return .red
        default: return .secondary
        }
    }
}

extension PaymentRecord {
    var confirmationStatusLabel: String {
        guard shouldShowConfirmationProgress,
              let blockHeight = txBlockHeight, blockHeight > 0 else { return "" }
        if confirmations >= ConfirmationPolicy.requiredConfirmations {
            return "Confirmed"
        }
        return "\(confirmations)/\(ConfirmationPolicy.requiredConfirmations) confirmations"
    }
}

private extension PaymentRecord {
    var shouldPreferUSDDisplay: Bool {
        switch paymentType {
        case "splice_in", "splice_out", "onchain", "channel_close":
            return true
        default:
            return false
        }
    }

    func displayUSD(fallbackPrice: Double) -> Double? {
        if let amountUSD {
            return amountUSD
        }
        guard shouldPreferUSDDisplay else { return nil }
        let price = (btcPrice ?? 0) > 0 ? (btcPrice ?? 0) : fallbackPrice
        guard price > 0 else { return nil }
        return Double(amountSats) / Double(Constants.satsInBTC) * price
    }
}
