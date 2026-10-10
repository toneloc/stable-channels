import SwiftUI
import UIKit

struct OnChainSendView: View {
    @Environment(AppState.self) private var appState
    @State private var address = ""
    @State private var amountUSDStr = ""
    @State private var sendAll = false
    @State private var isSending = false
    @State private var errorMessage: String?
    @State private var txid: String?
    @State private var spliceSuccess = false
    @State private var feeRateSatVb: Double?
    @State private var showReviewSheet = false
    @State private var selectedFeeTier: NetworkFeeSpeedTier = .standard

    private var amountSats: UInt64? {
        convertedSats(price: appState.accountingBTCPrice)
    }

    private func convertedSats(price: Double) -> UInt64? {
        guard let usd = Double(amountUSDStr), usd > 0, price > 0 else { return nil }
        let sats = usd / price * Double(Constants.satsInBTC)
        guard sats.isFinite, sats >= 1, sats < Double(UInt64.max) else { return nil }
        return UInt64(sats)
    }

    private var hasReadyChannel: Bool {
        appState.nodeService.channels.contains { $0.isChannelReady }
    }

    private var feeEstimateText: String {
        guard let feeRateSatVb else {
            return String(localized: "info_fee_estimating", defaultValue: "Estimating network fee...")
        }
        let feeSats = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: feeRateSatVb, isSendAll: sendAll)
        let rateDisplay = feeRateSatVb.isFinite && feeRateSatVb > 0
            ? (feeRateSatVb.truncatingRemainder(dividingBy: 1) == 0
                ? String(format: "%.0f", feeRateSatVb)
                : String(format: "%.1f", feeRateSatVb))
            : "1"
        return String(
            format: String(
                localized: "info_onchain_fee_estimate_sentence",
                defaultValue: "Expected network fee: ~%@ BTC (%@ sat/vB)"
            ),
            feeSats.btcSpacedFormatted,
            rateDisplay
        )
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    addressCard
                    OnChainAmountCard(
                        hasReadyChannel: hasReadyChannel,
                        sendAll: $sendAll,
                        amountUSDStr: $amountUSDStr,
                        amountSats: amountSats
                    )
                    infoCard(icon: "bitcoinsign.circle", text: feeEstimateText)
                    if hasReadyChannel {
                        infoCard(
                            icon: "arrow.up.arrow.down",
                            text: String(
                                localized: "info_splice_out_funds",
                                defaultValue: "Funds will be sent via splice-out from your Lightning channel."
                            )
                        )
                    }
                    if let txid {
                        successCard(
                            icon: "checkmark.circle.fill",
                            title: String(localized: "success_sent", defaultValue: "Sent!"),
                            detail: String(localized: "label_txid", defaultValue: "TXID") + ": " + txid,
                            monospaced: true,
                            linkStyle: true
                        )
                    }
                    if spliceSuccess {
                        successCard(
                            icon: "checkmark.circle.fill",
                            title: String(localized: "success_splice_out", defaultValue: "Splice-out initiated!"),
                            detail: String(
                                localized: "info_funds_arrive_onchain",
                                defaultValue: "Funds will arrive onchain after confirmation."
                            ),
                            monospaced: false
                        )
                    }
                    if let error = errorMessage {
                        errorCard(error)
                    }
                    reviewButton
                    Spacer(minLength: 12)
                }
                .padding(20)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Color(.systemGroupedBackground))
            .navigationTitle(String(localized: "title_send_on_chain", defaultValue: "Send Onchain"))
            .navigationBarTitleDisplayMode(.inline)
            .qrInputToolbar(text: $address, sanitize: QRCodeExtractor.sanitizeAddress)
            .sheet(isPresented: $showReviewSheet) {
                OnChainReviewSheet(
                    address: address,
                    sendAll: sendAll,
                    amountSats: amountSats,
                    feeRateSatVb: feeRateSatVb,
                    selectedFeeTier: $selectedFeeTier
                ) { sentTxid, isSplice in
                    if let sentTxid {
                        self.txid = sentTxid
                        self.spliceSuccess = false
                    } else if isSplice {
                        self.spliceSuccess = true
                        self.txid = nil
                    }
                }
            }
            .task {
                feeRateSatVb = await appState.feeRateService.currentRate()
            }
        }
    }

    private var addressCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: "wallet.bifold")
                    .foregroundStyle(.secondary)
                Text(String(localized: "header_destination_address", defaultValue: "Destination Address"))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
            }

            TextField(String(localized: "placeholder_address", defaultValue: "bc1..."), text: $address)
                .font(.system(.body, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(12)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
                .overlay(
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(Color.primary.opacity(0.08), lineWidth: 1)
                )
                .onChange(of: address) { _, new in
                    address = QRCodeExtractor.sanitizeAddress(new)
                }
        }
        .padding(16)
        .glassCard()
    }

    private func infoCard(icon: String, text: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: icon)
                .foregroundStyle(.secondary)
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
            Spacer()
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassCard()
    }

    private func successCard(icon: String, title: String, detail: String, monospaced: Bool,
                             linkStyle: Bool = false) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon)
                .font(.title2)
                .foregroundStyle(.green)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.headline)
                if linkStyle {
                    Text(makeTxidAttributed(label: "TXID: ", txid: detail.replacingOccurrences(of: "TXID: ", with: "")))
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                } else {
                    Text(detail)
                        .font(monospaced ? .system(.caption, design: .monospaced) : .caption)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }
            Spacer()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(.green.opacity(0.12))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16)
                .stroke(.green.opacity(0.3), lineWidth: 1)
        )
    }

    private func errorCard(_ message: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.title2)
                .foregroundStyle(.red)
            Text(message)
                .font(.subheadline)
                .foregroundStyle(.red)
            Spacer()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(.red.opacity(0.12))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16)
                .stroke(.red.opacity(0.3), lineWidth: 1)
        )
    }

    private var reviewButton: some View {
        Button {
            errorMessage = nil
            showReviewSheet = true
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.body.weight(.semibold))
                Text(String(localized: "button_review_transfer", defaultValue: "Review Transfer"))
                    .fontWeight(.semibold)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
        }
        .buttonStyle(.borderedProminent)
        .tint(.blue)
        .disabled(address.isEmpty || (!sendAll && (amountSats ?? 0) == 0))
        .animation(.easeInOut(duration: 0.2), value: address.isEmpty)
        .animation(.easeInOut(duration: 0.2), value: amountSats)
    }

    private func makeTxidAttributed(label: String, txid: String) -> AttributedString {
        var s = AttributedString(label)
        s.foregroundColor = .secondary
        var t = AttributedString(txid)
        t.foregroundColor = .blue
        t.underlineStyle = .single
        t.link = Constants.txExplorerLink(for: txid)
        return s + t
    }
}

private struct GlassCardModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(.ultraThinMaterial)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)
            )
    }
}

private extension View {
    func glassCard() -> some View { modifier(GlassCardModifier()) }
}
