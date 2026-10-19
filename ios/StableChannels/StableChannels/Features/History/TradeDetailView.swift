import SwiftUI

struct TradeDetailView: View {
    let trade: TradeRecord
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section(String(localized: .sectionTradeDetails)) {
                    row(String(localized: .labelAction),
                        trade.action == "buy"
                            ? String(localized: .tradeBuyBtc)
                            : String(localized: .tradeSellBtc))
                    row(
                        String(localized: .labelAmountUsd),
                        trade.amountUSD.usdFormatted
                    )
                    row(
                        String(localized: .labelAmountBtc),
                        Bitcoin.fromBTC(trade.amountBTC).formatted
                    )
                    row(String(localized: .labelBtcPrice), trade.btcPrice.usdFormatted)
                    row(String(localized: .labelFee), trade.feeUSD.usdFormatted)
                    row(String(localized: .labelStatus), trade.status.capitalized)
                }

                Section(String(localized: .sectionMetadata)) {
                    row(String(localized: .labelDate), trade.date.formatted())
                    if let paymentId = trade.paymentId {
                        row(String(localized: .labelPaymentId), paymentId)
                    }
                }
            }
            .navigationTitle(String(localized: .titleTradeDetail))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: .buttonDone)) { dismiss() }
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
