import SwiftUI

struct TradeDetailView: View {
    let trade: TradeRecord
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("Trade Details") {
                    row("Action", trade.action == "buy" ? "Buy BTC" : "Sell BTC")
                    row("Amount (USD)", String(format: "$%.2f", trade.amountUSD))
                    row("Amount (BTC)", String(format: "%.8f", trade.amountBTC))
                    row("BTC Price", String(format: "$%.2f", trade.btcPrice))
                    row("Fee", String(format: "$%.2f", trade.feeUSD))
                    row("Status", trade.status.capitalized)
                }

                Section("Metadata") {
                    row("Date", trade.date.formatted())
                    if let paymentId = trade.paymentId {
                        row("Payment ID", paymentId)
                    }
                }
            }
            .navigationTitle("Trade Detail")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label)
                .foregroundStyle(.secondary)
            Spacer()
            Text(value)
                .textSelection(.enabled)
        }
    }
}
