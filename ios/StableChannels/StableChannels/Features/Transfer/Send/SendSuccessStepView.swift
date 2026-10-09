import SwiftUI

/// Step 4: Payment completion receipt with transaction ID and dismiss controls.
struct SendSuccessStepView: View {
    @Bindable var model: SendFlowModel
    @Environment(AppState.self) private var appState
    let onDismiss: () -> Void

    @State private var hasAppeared = false
    @State private var rippleScale: CGFloat = 0.8
    @State private var rippleOpacity: Double = 0.65
    @State private var isCopied = false
    @State private var copyTask: Task<Void, Never>?

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            statusHeaderSection

            amountSection

            detailsSection

            Spacer()

            Button {
                onDismiss()
            } label: {
                Text(String(localized: "button_done", defaultValue: "Done"))
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .tint(Color.blue)
            .padding(.bottom, 16)
        }
        .padding(.horizontal, 20)
        .padding(.top, 16)
        .onAppear {
            withAnimation(.spring(response: 0.45, dampingFraction: 0.65)) {
                hasAppeared = true
            }
            withAnimation(.easeOut(duration: 0.85)) {
                rippleScale = 1.45
                rippleOpacity = 0.0
            }
            UINotificationFeedbackGenerator().notificationOccurred(
                model.isPendingSettlement ? .warning : .success
            )
        }
        .onDisappear {
            copyTask?.cancel()
        }
    }

    private var statusHeaderSection: some View {
        ZStack {
            Circle()
                .stroke(statusColor.opacity(rippleOpacity), lineWidth: 2)
                .frame(width: 90, height: 90)
                .scaleEffect(rippleScale)

            Circle()
                .fill(statusColor.opacity(0.15))
                .frame(width: 90, height: 90)
                .scaleEffect(hasAppeared ? 1.0 : 0.4)

            Image(systemName: statusIconName)
                .font(.system(size: 64))
                .foregroundStyle(statusColor)
                .scaleEffect(hasAppeared ? 1.0 : 0.2)
                .rotationEffect(hasAppeared ? .degrees(0) : .degrees(-30))
        }
    }

    private var amountSection: some View {
        VStack(spacing: 6) {
            Text(verbatim: successTitle)
                .font(.title2.weight(.bold))

            let sats = model.sentAmountSats
            let usd = (Double(sats) / Double(Constants.satsInBTC)) * appState.btcPrice
            if appState.btcPrice > 0 {
                Text(usd.usdFormatted)
                    .font(.system(size: 28, weight: .bold, design: .rounded))
            }
            Text("\(sats.btcSpacedFormatted) BTC")
                .font(.subheadline)
                .foregroundStyle(.secondary)

            if model.isPendingSettlement {
                Text(String(
                    localized: "note_payment_pending",
                    defaultValue: "Payment dispatched. Settlement is pending in the background. Check History for final status."
                ))
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 4)
                .padding(.horizontal, 16)
            } else if isBolt12 {
                Text(String(
                    localized: "note_bolt12_asynchronous",
                    defaultValue: "Offer payment dispatched. Your node is requesting an invoice over Lightning onion messaging."
                ))
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 4)
                .padding(.horizontal, 16)
            }
        }
    }

    private var detailsSection: some View {
        VStack(spacing: 8) {
            if let txid = model.successTxid {
                transactionDetailsCard(id: txid, isTxid: true)
            } else if let paymentId = model.successPaymentId {
                transactionDetailsCard(id: paymentId, isTxid: false)
            }

            if let txid = model.successTxid, let explorerURL = Constants.txExplorerLink(for: txid) {
                Link(destination: explorerURL) {
                    HStack(spacing: 6) {
                        Text(String(localized: "button_view_on_explorer", defaultValue: "View on Block Explorer"))
                            .font(.caption.weight(.medium))
                        Image(systemName: "arrow.up.right")
                            .font(.caption2.weight(.semibold))
                    }
                    .foregroundStyle(Color.blue)
                    .padding(.top, 4)
                }
            }
        }
    }

    private func transactionDetailsCard(id: String, isTxid: Bool) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            if let recipient = recipientDisplay {
                VStack(alignment: .leading, spacing: 4) {
                    Text(String(localized: "label_recipient_caps", defaultValue: "RECIPIENT"))
                        .font(.system(size: 11, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)

                    Text(recipient)
                        .font(.system(.footnote, design: .monospaced))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Divider()
                    .padding(.vertical, 10)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text(
                    isTxid
                        ? String(localized: "label_txid_caps", defaultValue: "TRANSACTION ID")
                        : String(localized: "label_payment_id_caps", defaultValue: "PAYMENT ID")
                )
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .foregroundStyle(.secondary)

                HStack(spacing: 8) {
                    Text(id)
                        .font(.system(.footnote, design: .monospaced))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    Spacer()

                    Button {
                        copyId(id)
                    } label: {
                        Image(systemName: isCopied ? "checkmark" : "doc.on.doc")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(isCopied ? Color.green : Color.secondary)
                            .contentTransition(.symbolEffect(.replace))
                            .frame(width: 28, height: 28)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: 12, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.primary.opacity(0.08), lineWidth: 1)
        )
        .contentShape(Rectangle())
        .onTapGesture {
            copyId(id)
        }
    }

    private func copyId(_ id: String) {
        UIPasteboard.general.string = id
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        copyTask?.cancel()
        withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) {
            isCopied = true
        }
        copyTask = Task {
            try? await Task.sleep(nanoseconds: 1_800_000_000)
            guard !Task.isCancelled else { return }
            withAnimation(.easeInOut(duration: 0.25)) {
                isCopied = false
            }
        }
    }

    private var recipientDisplay: String? {
        guard let dest = model.destination else { return nil }
        switch dest {
        case .onchain(let address, _):
            return address
        case .lightningAddress(let handle, let domain, _):
            return "\(handle)@\(domain)"
        case .lnurlPay(let url):
            return url.host ?? url.absoluteString
        case .bolt11, .bolt12:
            return dest.displayTitle
        }
    }

    private var isBolt12: Bool {
        if case .bolt12 = model.destination { return true }
        return false
    }

    private var statusColor: Color {
        model.isPendingSettlement ? .orange : .green
    }

    private var statusIconName: String {
        model.isPendingSettlement ? "hourglass" : "checkmark.circle.fill"
    }

    private var successTitle: String {
        if model.isPendingSettlement {
            return String(localized: "title_payment_pending", defaultValue: "Payment Pending")
        } else if isBolt12 {
            return String(localized: "title_payment_initiated", defaultValue: "Payment Initiated")
        } else {
            return String(localized: "title_payment_sent", defaultValue: "Payment Sent")
        }
    }
}

#Preview("Settled Payment - Lightning (Light)") {
    let model = SendFlowModel()
    model.step = .success
    model.destination = .lightningAddress(
        handle: "prabal",
        domain: "0xprabal.com",
        url: URL(string: "https://0xprabal.com/.well-known/lnurlp/prabal")!
    )
    model.sentAmountSats = 50_000
    model.successPaymentId = "f4a22400938b8120c1928374a8d9b1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8"
    return ZStack {
        Color(uiColor: .systemGroupedBackground).ignoresSafeArea()
        SendSuccessStepView(model: model) {}
    }
    .preferredColorScheme(.light)
    .environment(AppState())
}

#Preview("Settled Payment - Lightning (Dark)") {
    let model = SendFlowModel()
    model.step = .success
    model.destination = .lightningAddress(
        handle: "prabal",
        domain: "0xprabal.com",
        url: URL(string: "https://0xprabal.com/.well-known/lnurlp/prabal")!
    )
    model.sentAmountSats = 50_000
    model.successPaymentId = "f4a22400938b8120c1928374a8d9b1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8"
    return ZStack {
        Color(uiColor: .systemGroupedBackground).ignoresSafeArea()
        SendSuccessStepView(model: model) {}
    }
    .preferredColorScheme(.dark)
    .environment(AppState())
}

#Preview("Settled Payment - On-Chain") {
    let model = SendFlowModel()
    model.step = .success
    model.destination = .onchain(
        address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
        amountSats: 250_000
    )
    model.sentAmountSats = 250_000
    model.successTxid = "9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b"
    return ZStack {
        Color(uiColor: .systemGroupedBackground).ignoresSafeArea()
        SendSuccessStepView(model: model) {}
    }
    .environment(AppState())
}

#Preview("Pending Settlement") {
    let model = SendFlowModel()
    model.step = .success
    model.destination = .lightningAddress(
        handle: "alice",
        domain: "domain.com",
        url: URL(string: "https://domain.com")!
    )
    model.sentAmountSats = 125_000
    model.isPendingSettlement = true
    model.successPaymentId = "3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d"
    return ZStack {
        Color(uiColor: .systemGroupedBackground).ignoresSafeArea()
        SendSuccessStepView(model: model) {}
    }
    .environment(AppState())
}
