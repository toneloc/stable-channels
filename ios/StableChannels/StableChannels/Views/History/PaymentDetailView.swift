import SwiftUI

struct PaymentDetailView: View {
    let payment: PaymentRecord
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("Payment Details") {
                    row("Direction", payment.isIncoming ? "Received" : "Sent")
                    row("Type", paymentTypeLabel)
                    row("Amount", "\(payment.amountSats) sats")
                    if let usd = payment.amountUSD {
                        row("USD Value", usd.usdFormatted)
                    }
                    if let price = payment.btcPrice {
                        row("BTC Price", price.usdFormatted)
                    }
                    if payment.feeMsat > 0 {
                        row("Fee", "\(payment.feeMsat) msat")
                    }
                    row("Status", payment.status.capitalized)
                }

                Section("Metadata") {
                    row("Date", payment.date.formatted())
                    if let paymentId = payment.paymentId {
                        row("Payment ID", paymentId)
                    }
                    if let txid = payment.txid {
                        row("TXID", txid)
                    }
                    if let address = payment.address {
                        row("Address", address)
                    }
                    if payment.confirmations > 0 {
                        row("Confirmations", "\(payment.confirmations)")
                    }
                }
            }
            .navigationTitle("Payment Detail")
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

    private var paymentTypeLabel: String {
        switch payment.paymentType {
        case "stability": return "Stability Payment"
        case "lightning": return "Settlement"
        case "splice_in": return "Splice In"
        case "splice_out": return "Splice Out"
        case "onchain": return "On-chain"
        case "bolt12": return "Bolt12"
        default: return payment.paymentType
        }
    }
}
