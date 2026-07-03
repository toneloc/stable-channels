import SwiftUI

struct PaymentDetailView: View {
    let payment: PaymentRecord
    let displayPrice: Double
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section(String(localized: "section_payment_details", defaultValue: "Payment Details")) {
                    row(String(localized: "label_direction", defaultValue: "Direction"),
                        payment.isIncoming
                            ? String(localized: "payment_received", defaultValue: "Received")
                            : String(localized: "payment_sent", defaultValue: "Sent"))
                    row(String(localized: "label_type", defaultValue: "Type"), paymentTypeLabel)
                    row(
                        String(localized: "label_amount", defaultValue: "Amount"),
                        "\(payment.amountSats.btcSpacedFormatted) BTC"
                    )
                    if let usd = displayUSD {
                        row(String(localized: "label_usd_value", defaultValue: "USD Value"), usd.usdFormatted)
                    }
                    if let price = payment.btcPrice {
                        row(String(localized: "label_btc_price", defaultValue: "BTC Price"), price.usdFormatted)
                    }
                    if payment.feeMsat > 0 {
                        row(String(localized: "label_fee", defaultValue: "Fee"), "\(payment.feeMsat) msat")
                    }
                    row(String(localized: "label_status", defaultValue: "Status"), payment.status.capitalized)
                }

                Section(String(localized: "section_metadata", defaultValue: "Metadata")) {
                    row(String(localized: "label_date", defaultValue: "Date"), payment.date.formatted())
                    if let paymentId = payment.paymentId {
                        row(String(localized: "label_payment_id", defaultValue: "Payment ID"), paymentId)
                    }
                    if let txid = payment.txid {
                        row(String(localized: "label_txid", defaultValue: "TXID"), txid)
                        if let url = Constants.txExplorerLink(for: txid) {
                            Link(destination: url) {
                                HStack(spacing: 4) {
                                    Text(String(localized: "view_on_explorer", defaultValue: "View on explorer"))
                                    Image(systemName: "arrow.up.right.square")
                                }
                                .font(.caption)
                            }
                        }
                    }
                    if let address = payment.address {
                        row(String(localized: "label_address", defaultValue: "Address"), address)
                    }
                    if payment.confirmations > 0 {
                        row(
                            String(localized: "label_confirmations", defaultValue: "Confirmations"),
                            "\(payment.confirmations)"
                        )
                    }
                }
            }
            .navigationTitle(String(localized: "title_payment_detail", defaultValue: "Payment Detail"))
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

    private var paymentTypeLabel: String {
        switch payment.paymentType {
        case "stability": return String(localized: "payment_type_stability", defaultValue: "Stability Payment")
        case "lightning": return String(localized: "payment_type_settlement", defaultValue: "Settlement")
        case "splice_in": return String(localized: "payment_type_splice_in", defaultValue: "Splice In")
        case "splice_out": return String(localized: "payment_type_splice_out", defaultValue: "Splice Out")
        case "onchain": return String(localized: "payment_type_on_chain", defaultValue: "Onchain")
        case "channel_close": return String(localized: "payment_type_channel_close", defaultValue: "Channel Close")
        case "bolt12": return String(localized: "payment_type_bolt12", defaultValue: "Bolt12")
        default: return payment.paymentType
        }
    }

    private var displayUSD: Double? {
        if let amountUSD = payment.amountUSD { return amountUSD }
        guard shouldPreferUSDDisplay else { return nil }
        let price = (payment.btcPrice ?? 0) > 0 ? (payment.btcPrice ?? 0) : displayPrice
        guard price > 0 else { return nil }
        return Double(payment.amountSats) / Double(Constants.satsInBTC) * price
    }

    private var shouldPreferUSDDisplay: Bool {
        switch payment.paymentType {
        case "splice_in", "splice_out", "onchain", "channel_close":
            return true
        default:
            return false
        }
    }
}
