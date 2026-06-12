import SwiftUI

struct TradeDetailView: View {
    let trade: TradeRecord
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section(String(localized: "section_trade_details", defaultValue: "Order Details")) {
                    row(String(localized: "label_action", defaultValue: "Action"),
                        trade.action == "buy"
                            ? String(localized: "trade_buy_btc", defaultValue: "USD → BTC")
                            : String(localized: "trade_sell_btc", defaultValue: "BTC → USD"))
                    row(
                        String(localized: "label_amount_usd", defaultValue: "Amount (USD)"),
                        trade.amountUSD.usdFormatted
                    )
                    row(
                        String(localized: "label_amount_btc", defaultValue: "Amount (BTC)"),
                        "\(UInt64(trade.amountBTC * Double(Constants.satsInBTC)).btcSpacedFormatted) BTC"
                    )
                    row(String(localized: "label_btc_price", defaultValue: "BTC Price"), trade.btcPrice.usdFormatted)
                    row(String(localized: "label_fee", defaultValue: "Fee"), trade.feeUSD.usdFormatted)
                    row(String(localized: "label_status", defaultValue: "Status"), trade.status.capitalized)
                }

                Section(String(localized: "section_metadata", defaultValue: "Metadata")) {
                    row(String(localized: "label_date", defaultValue: "Date"), trade.date.formatted())
                    if let paymentId = trade.paymentId {
                        row(String(localized: "label_payment_id", defaultValue: "Order ID"), paymentId)
                    }
                }
            }
            .navigationTitle(String(localized: "title_trade_detail", defaultValue: "Order Detail"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_done", defaultValue: "Done")) { dismiss() }
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
