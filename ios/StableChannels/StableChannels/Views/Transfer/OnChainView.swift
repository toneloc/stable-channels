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

    private var amountSats: UInt64? {
        guard let usd = Double(amountUSDStr), usd > 0, appState.btcPrice > 0 else { return nil }
        return UInt64(usd / appState.btcPrice * Double(Constants.satsInBTC))
    }

    private var hasReadyChannel: Bool {
        appState.nodeService.channels.contains { $0.isChannelReady }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    addressCard
                    amountCard
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
                                defaultValue: "Funds will arrive on-chain after confirmation."
                            ),
                            monospaced: false
                        )
                    }
                    if let error = errorMessage {
                        errorCard(error)
                    }
                    sendButton
                    Spacer(minLength: 12)
                }
                .padding(20)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Color(.systemGroupedBackground))
            .navigationTitle(String(localized: "title_send_on_chain", defaultValue: "Send On-Chain"))
            .navigationBarTitleDisplayMode(.inline)
            .qrInputToolbar(text: $address, sanitize: QRCodeExtractor.sanitizeAddress)
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

    private var amountCard: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Image(systemName: "dollarsign.circle")
                    .foregroundStyle(.secondary)
                Text(String(localized: "header_amount", defaultValue: "Amount"))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
            }

            if sendAll {
                Label(
                    String(localized: "label_all_available_funds", defaultValue: "All available funds"),
                    systemImage: "infinity"
                )
                .font(.headline)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8)
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(String(localized: "label_dollar_sign", defaultValue: "$"))
                        .font(.system(size: 36, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                    TextField(
                        String(localized: "placeholder_amount_usd", defaultValue: "0.00"),
                        text: $amountUSDStr
                    )
                    .keyboardType(.decimalPad)
                    .font(.system(size: 36, weight: .semibold, design: .rounded))
                    .onChange(of: amountUSDStr) { _, new in
                        amountUSDStr = InputSanitizer.decimal(new)
                    }
                }
                if let sats = amountSats, sats > 0 {
                    HStack(spacing: 6) {
                        Image(systemName: "bitcoinsign.circle.fill")
                            .foregroundStyle(.primary)
                        Text("\(sats.btcSpacedFormatted) BTC")
                            .font(.subheadline.weight(.medium))
                            .contentTransition(.numericText())
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.ultraThinMaterial, in: Capsule())
                    .animation(.snappy, value: sats)
                }
            }

            Toggle(isOn: $sendAll) {
                Label(
                    String(localized: "toggle_send_all", defaultValue: "Send All"),
                    systemImage: "infinity.circle"
                )
                .font(.subheadline)
            }
            .tint(.green)
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

    private var sendButton: some View {
        Button {
            Task { await send() }
        } label: {
            HStack(spacing: 8) {
                if isSending {
                    ProgressView()
                        .tint(.white)
                } else {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.body.weight(.semibold))
                    Text(String(localized: "button_send_payment", defaultValue: "Send"))
                        .fontWeight(.semibold)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
        }
        .buttonStyle(.borderedProminent)
        .tint(.blue)
        .disabled(address.isEmpty || (!sendAll && (amountSats ?? 0) == 0) || isSending)
        .scaleEffect(isSending ? 0.97 : 1.0)
        .animation(.spring(response: 0.3, dampingFraction: 0.7), value: isSending)
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

    private func send() async {
        // Dismiss any active keyboard to avoid blocking system auth dialogs
        UIApplication.shared.sendAction(
            Selector(("resignFirstResponder")),
            to: nil,
            from: nil,
            for: nil
        )

        // Always require auth for on-chain sends — highest risk, drains to external wallet
        let authReason = sendAll ? "Confirm on-chain withdrawal" : "Confirm on-chain send"
        let authPassed = await appState.authenticate(reason: authReason)
        guard authPassed else {
            errorMessage = appState.authError ?? "Authentication required to send."
            return
        }

        isSending = true
        errorMessage = nil
        defer { isSending = false }

        let sats = sendAll ? UInt64(0) : (amountSats ?? 0)
        guard sendAll || sats > 0 else { return }

        do {
            // If channel exists, route through splice-out
            if let channel = appState.nodeService.channels.first(where: { $0.isChannelReady }), !sendAll {
                guard !appState.isSweeping else {
                    throw NSError(
                        domain: "",
                        code: 0,
                        userInfo: [NSLocalizedDescriptionKey: String(
                            localized: "error_splice_in_progress",
                            defaultValue: "A splice is already in progress — try again shortly"
                        )]
                    )
                }
                try appState.nodeService.spliceOut(
                    userChannelId: channel.userChannelId,
                    counterpartyNodeId: channel.counterpartyNodeId,
                    address: address,
                    amountSats: sats
                )
                appState.pendingSplice = PendingSplice(direction: "out", amountSats: sats, address: address)
                spliceSuccess = true
            } else if sendAll {
                let result = try appState.nodeService.sendAllOnchain(address: address)
                txid = result
                let price = appState.btcPrice
                let onchainSats = appState.onchainBalanceSats
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: result,
                    paymentType: "onchain",
                    direction: "sent",
                    amountMsat: onchainSats * 1000,
                    amountUSD: price > 0 ? Double(onchainSats) / Double(Constants.satsInBTC) * price : nil,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending",
                    txid: result,
                    address: address
                )
            } else {
                let result = try appState.nodeService.sendOnchain(address: address, amountSats: sats)
                txid = result
                let price = appState.btcPrice
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: result,
                    paymentType: "onchain",
                    direction: "sent",
                    amountMsat: sats * 1000,
                    amountUSD: price > 0 ? Double(sats) / Double(Constants.satsInBTC) * price : nil,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending",
                    txid: result,
                    address: address
                )
            }
        } catch {
            errorMessage = error.localizedDescription
        }
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
