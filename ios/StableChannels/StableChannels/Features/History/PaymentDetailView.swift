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
                        String(localized: .paymentDetailErrorTitle),
                        systemImage: "exclamationmark.triangle",
                        description: Text(String(localized: .paymentDetailErrorDesc))
                    )
                } else {
                    ProgressView()
                }
            }
            .navigationTitle(String(localized: .titlePaymentDetail))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: .buttonDone)) { dismiss() }
                }
            }
            .task {
                await loadPayment()
            }
            .onChange(of: appState.confirmationUpdateEpoch) { _, _ in
                Task { await loadPayment() }
            }
        }
    }

    @MainActor
    private func loadPayment() async {
        guard let db = appState.databaseService else { return }
        do {
            payment = try db.paymentRepo.getPayment(byId: paymentId)
            loadError = false
        } catch {
            loadError = true
        }
    }

    private func paymentList(_ payment: PaymentRecord) -> some View {
        List {
            Section(String(localized: .sectionPaymentDetails)) {
                row(String(localized: .labelDirection),
                    payment.isIncoming
                        ? String(localized: .paymentReceived)
                        : String(localized: .paymentSent))
                row(String(localized: .labelType), paymentTypeLabel)
                row(
                    String(localized: .labelAmount),
                    "\(payment.amountSats.btcSpacedFormatted) BTC"
                )
                if let usd = displayUSD {
                    row(String(localized: .labelUsdValue), usd.usdFormatted)
                }
                if let price = payment.btcPrice {
                    row(String(localized: .labelBtcPrice), price.usdFormatted)
                }
                if payment.feeMsat > 0 {
                    row(
                        String(localized: .labelFee),
                        "\((payment.feeMsat / 1000).btcSpacedFormatted) BTC"
                    )
                }
                row(String(localized: .labelStatus), statusLabel(for: payment))
                if payment.shouldShowConfirmationProgress {
                    confirmationProgressRow(for: payment)
                } else if payment.confirmations > 0 {
                    row(
                        String(localized: .labelConfirmations),
                        "\(payment.confirmations)"
                    )
                }
            }

            Section(String(localized: .sectionMetadata)) {
                row(String(localized: .labelDate), payment.date.formatted())
                if let pId = payment.paymentId {
                    row(String(localized: .labelPaymentId), pId)
                }
                if let txid = payment.txid {
                    row(String(localized: .labelTxid), txid)
                    if let url = Constants.txExplorerLink(for: txid) {
                        Link(destination: url) {
                            HStack(spacing: 4) {
                                Text(String(localized: .viewOnExplorer))
                                Image(systemName: "arrow.up.right.square")
                            }
                            .font(.caption)
                        }
                    }
                }
                if let address = payment.address {
                    row(String(localized: .labelAddress), address)
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

    private func statusLabel(for payment: PaymentRecord) -> String {
        if payment.shouldShowConfirmationProgress {
            return payment.confirmationProgress.label
        }
        // 'expired' is a DB-level state (splice initiation swept by the
        // no-txid timeout, still recoverable by a late negotiation); to the
        // user it is indistinguishable from a failed splice.
        if payment.status == "expired" {
            return "failed".capitalized
        }
        return payment.status.capitalized
    }

    @ViewBuilder
    private func confirmationProgressRow(for payment: PaymentRecord) -> some View {
        let progress = payment.confirmationProgress
        HStack {
            Text(String(localized: .labelConfirmations))
                .foregroundStyle(.secondary)
            Spacer()
            if progress.isComplete {
                Label(
                    String(localized: .confirmationsComplete),
                    systemImage: "checkmark.circle.fill"
                )
                .foregroundStyle(.green)
                .font(.subheadline)
            } else {
                HStack(spacing: 8) {
                    Text("\(progress.display)/\(progress.required)")
                        .font(.subheadline)
                    ProgressView(value: Double(progress.display), total: Double(progress.required))
                        .frame(width: 60)
                        .tint(progress.display > 0 ? .blue : .orange)
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
        case "stability": return String(localized: .paymentTypeStability)
        case "lightning": return String(localized: .paymentTypeLightning)
        case "splice_in": return String(localized: .paymentTypeSpliceIn)
        case "splice_out": return String(localized: .paymentTypeSpliceOut)
        case "onchain": return String(localized: .paymentTypeOnChain)
        case "channel_close": return String(localized: .paymentTypeChannelClose)
        case "bolt12": return String(localized: .paymentTypeBolt12)
        default: return payment?.paymentType ?? ""
        }
    }
}

#Preview {
    PaymentDetailView(paymentId: 0, displayPrice: 0)
        .environment(AppState())
}
