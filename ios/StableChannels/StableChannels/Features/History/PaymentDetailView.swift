import SwiftUI

struct PaymentDetailView: View {
    let paymentId: Int64
    let displayPrice: Double
    @Environment(\.dismiss) private var dismiss
    @Environment(AppState.self) private var appState
    @State private var payment: PaymentRecord?
    @State private var loadError: Bool = false

    var body: some View {
        NavigationStack {
            Group {
                if let payment {
                    paymentList(payment)
                } else if loadError {
                    ContentUnavailableView(
                        String(localized: "payment_detail_error_title", defaultValue: "Error"),
                        systemImage: "exclamationmark.triangle",
                        description: Text(String(
                            localized: "payment_detail_error_desc",
                            defaultValue: "Could not load payment details."
                        ))
                    )
                } else {
                    ProgressView()
                }
            }
            .navigationTitle(String(localized: "title_payment_detail", defaultValue: "Payment Detail"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_done", defaultValue: "Done")) { dismiss() }
                }
            }
            .task {
                await loadPayment()
                for await _ in Timer.publish(every: 30, on: .main, in: .common).autoconnect().values {
                    await loadPayment()
                }
            }
        }
    }

    @MainActor
    private func loadPayment() async {
        guard let db = appState.databaseService else { return }
        do {
            payment = try db.getPayment(byId: paymentId)
            loadError = false
        } catch {
            loadError = true
        }
    }

    private func paymentList(_ payment: PaymentRecord) -> some View {
        List {
            Section(String(localized: "section_payment_details", defaultValue: "Payment Details")) {
                row(String(localized: "label_direction", defaultValue: "Direction"),
                    payment.isIncoming
                        ? String(localized: "payment_received", defaultValue: "Received")
                        : String(localized: "payment_sent", defaultValue: "Sent"))
                row(String(localized: "label_type", defaultValue: "Type"), paymentTypeLabel)
                row(String(localized: "label_amount", defaultValue: "Amount"), "\(payment.amountSats) sats")
                if let usd = displayUSD {
                    row(String(localized: "label_usd_value", defaultValue: "USD Value"), usd.usdFormatted)
                }
                if let price = payment.btcPrice {
                    row(String(localized: "label_btc_price", defaultValue: "BTC Price"), price.usdFormatted)
                }
                if payment.feeMsat > 0 {
                    row(String(localized: "label_fee", defaultValue: "Fee"), "\(payment.feeMsat) msat")
                }
                row(String(localized: "label_status", defaultValue: "Status"), statusLabel(for: payment))
            }

            Section(String(localized: "section_metadata", defaultValue: "Metadata")) {
                row(String(localized: "label_date", defaultValue: "Date"), payment.date.formatted())
                if let pId = payment.paymentId {
                    row(String(localized: "label_payment_id", defaultValue: "Payment ID"), pId)
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
                if payment.shouldShowConfirmationProgress {
                    confirmationProgressRow(for: payment)
                } else if payment.confirmations > 0 {
                    row(
                        String(localized: "label_confirmations", defaultValue: "Confirmations"),
                        "\(payment.confirmations)"
                    )
                }
            }
        }
        .listStyle(.plain)
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

    private func statusLabel(for payment: PaymentRecord) -> String {
        if payment.shouldShowConfirmationProgress {
            let confs = Int(payment.confirmations)
            if confs >= ConfirmationPolicy.requiredConfirmations {
                return String(localized: "status_confirmed", defaultValue: "Confirmed")
            } else if confs > 0 {
                return "\(confs)/\(ConfirmationPolicy.requiredConfirmations) confirmed"
            }
        }
        return payment.status.capitalized
    }

    @ViewBuilder
    private func confirmationProgressRow(for payment: PaymentRecord) -> some View {
        let confs = min(Int(payment.confirmations), ConfirmationPolicy.requiredConfirmations)
        let required = ConfirmationPolicy.requiredConfirmations
        HStack {
            Text(String(localized: "label_confirmations", defaultValue: "Confirmations"))
                .foregroundStyle(.secondary)
            Spacer()
            if confs >= required {
                Label(
                    String(localized: "confirmations_complete", defaultValue: "Confirmed"),
                    systemImage: "checkmark.circle.fill"
                )
                .foregroundStyle(.green)
                .font(.subheadline)
            } else {
                HStack(spacing: 8) {
                    Text("\(confs)/\(required)")
                        .font(.subheadline)
                    ProgressView(value: Double(confs), total: Double(required))
                        .frame(width: 60)
                        .tint(confs > 0 ? .blue : .orange)
                }
            }
        }
    }

    private var displayUSD: Double? {
        guard let payment else { return nil }
        if let amountUSD = payment.amountUSD {
            return amountUSD
        }
        guard shouldPreferUSDDisplay else { return nil }
        let price = (payment.btcPrice ?? 0) > 0 ? (payment.btcPrice ?? 0) : displayPrice
        guard price > 0 else { return nil }
        return Double(payment.amountSats) / Double(Constants.satsInBTC) * price
    }

    private var shouldPreferUSDDisplay: Bool {
        switch payment?.paymentType {
        case "splice_in", "splice_out", "onchain", "channel_close": return true
        default: return false
        }
    }

    private var paymentTypeLabel: String {
        switch payment?.paymentType {
        case "stability": return String(localized: "payment_type_stability", defaultValue: "Stability")
        case "lightning": return String(localized: "payment_type_lightning", defaultValue: "Lightning")
        case "splice_in": return String(localized: "payment_type_splice_in", defaultValue: "Splice In")
        case "splice_out": return String(localized: "payment_type_splice_out", defaultValue: "Splice Out")
        case "onchain": return String(localized: "payment_type_on_chain", defaultValue: "Onchain")
        case "channel_close": return String(localized: "payment_type_channel_close", defaultValue: "Channel Close")
        case "bolt12": return String(localized: "payment_type_bolt12", defaultValue: "Bolt12")
        default: return payment?.paymentType ?? ""
        }
    }
}

#Preview {
    PaymentDetailView(paymentId: 0, displayPrice: 0)
        .environment(AppState())
}
