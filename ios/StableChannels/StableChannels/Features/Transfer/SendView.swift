import SwiftUI
import UIKit
import LDKNode
import CoreImage

struct SendView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var input = ""
    @State private var amountSats = ""
    @State private var amountUSDStr = ""
    @State private var isSending = false
    @State private var errorMessage: String?
    @State private var success = false
    @State private var sentAmountSats: UInt64 = 0
    @State private var qrAlertMessage = ""

    private enum InputType {
        case bolt11
        case bolt12
        case onchain
        case unknown
    }

    private var detectedType: InputType {
        var trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if trimmed.hasPrefix("bitcoin:") {
            trimmed = String(trimmed.dropFirst(8))
        }
        if trimmed.hasPrefix("lnbc") || trimmed.hasPrefix("lntb") || trimmed.hasPrefix("lnts") {
            return .bolt11
        } else if trimmed.hasPrefix("lno") {
            return .bolt12
        } else if trimmed.hasPrefix("bc1") || trimmed.hasPrefix("1") || trimmed.hasPrefix("3") || trimmed
            .hasPrefix("tb1") || trimmed.hasPrefix("bcrt1") {
            // bcrt1 = regtest (E2E harness); the node validates network at send.
            return .onchain
        }
        return .unknown
    }

    /// Try to parse a bolt11 invoice amount from the current input
    private var parsedBolt11Msat: UInt64? {
        guard detectedType == .bolt11 else { return nil }
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard let inv = try? Bolt11Invoice.fromStr(invoiceStr: trimmed) else { return nil }
        return inv.amountMilliSatoshis()
    }

    private var isAmountlessBolt11: Bool {
        detectedType == .bolt11 && parsedBolt11Msat == nil &&
            !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var manualAmountMsat: UInt64 {
        guard appState.btcPrice > 0, let usd = Double(amountUSDStr), usd > 0 else { return 0 }
        let btc = usd / appState.btcPrice
        return UInt64(btc * Double(Constants.satsInBTC) * 1000)
    }

    /// Sats being sent (from invoice or manual entry)
    private var displaySats: UInt64 {
        switch detectedType {
        case .bolt11:
            if let msat = parsedBolt11Msat, msat > 0 {
                return msat / 1000
            }
            return manualAmountMsat / 1000
        case .bolt12, .onchain:
            guard let usd = Double(amountSats), usd > 0, appState.btcPrice > 0 else { return 0 }
            return UInt64(usd / appState.btcPrice * Double(Constants.satsInBTC))
        case .unknown:
            return 0
        }
    }

    private var displayUSD: Double? {
        let price = appState.btcPrice
        guard price > 0, displaySats > 0 else { return nil }
        return Double(displaySats) / Double(Constants.satsInBTC) * price
    }

    var body: some View {
        NavigationStack {
            Form {
                Section(String(localized: "header_invoice_address", defaultValue: "Invoice, Offer, or Address")) {
                    TextField(
                        String(localized: "placeholder_invoice",
                               defaultValue: "Paste invoice, bolt12 offer, or address..."),
                        text: $input,
                        axis: .vertical
                    )
                    .font(.system(.body, design: .monospaced))
                    .lineLimit(3...6)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                }

                if !input.trimmingCharacters(in: .whitespaces).isEmpty {
                    Section {
                        switch detectedType {
                        case .bolt11:
                            Label(
                                String(localized: "label_bolt11", defaultValue: "Bolt11 Invoice"),
                                systemImage: "bolt.fill"
                            )
                            .foregroundStyle(.blue)
                            if let msat = parsedBolt11Msat, msat > 0 {
                                let sats = msat / 1000
                                HStack {
                                    Text(String(localized: "label_amount_row", defaultValue: "Amount"))
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    VStack(alignment: .trailing, spacing: 2) {
                                        if let usd = displayUSD {
                                            Text(usd.usdFormatted)
                                                .fontWeight(.medium)
                                        }
                                        Text("\(sats.btcSpacedFormatted) BTC")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                HStack {
                                    Text(String(localized: "label_fee_row", defaultValue: "Fee"))
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    Text(String(localized: "info_fee_approx", defaultValue: "< 1%"))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            } else if isAmountlessBolt11 {
                                TextField(
                                    String(localized: "placeholder_amount_usd", defaultValue: "Amount (USD)"),
                                    text: $amountUSDStr
                                )
                                .keyboardType(.decimalPad)
                                if manualAmountMsat > 0 {
                                    HStack {
                                        Text(String(localized: "label_amount_row", defaultValue: "Amount"))
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                        VStack(alignment: .trailing, spacing: 2) {
                                            if let usd = displayUSD {
                                                Text(usd.usdFormatted)
                                                    .fontWeight(.medium)
                                            }
                                            Text("\(displaySats.btcSpacedFormatted) BTC")
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                    HStack {
                                        Text(String(localized: "label_fee_row", defaultValue: "Fee"))
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                        Text(String(localized: "info_fee_approx", defaultValue: "< 1%"))
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                            }
                        case .bolt12:
                            Label(
                                String(localized: "label_bolt12_offer", defaultValue: "Bolt12 Offer"),
                                systemImage: "bolt.fill"
                            )
                            .foregroundStyle(.purple)
                            TextField(
                                String(localized: "placeholder_amount_usd", defaultValue: "Amount (USD)"),
                                text: $amountSats
                            )
                            .keyboardType(.decimalPad)
                            .autocorrectionDisabled()
                            if let usd = displayUSD {
                                HStack {
                                    Text(String(localized: "label_amount", defaultValue: "Amount"))
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    VStack(alignment: .trailing, spacing: 2) {
                                        Text(usd.usdFormatted)
                                            .fontWeight(.medium)
                                        Text("\(displaySats.btcSpacedFormatted) BTC")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                HStack {
                                    Text(String(localized: "label_fee", defaultValue: "Fee"))
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    Text(String(localized: "info_fee_approx", defaultValue: "< 1%"))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        case .onchain:
                            Label(
                                String(localized: "label_on_chain_address", defaultValue: "Onchain Address"),
                                systemImage: "link"
                            )
                            .foregroundStyle(.orange)
                            TextField(
                                String(localized: "placeholder_amount_usd", defaultValue: "Amount (USD)"),
                                text: $amountSats
                            )
                            .keyboardType(.decimalPad)
                            .autocorrectionDisabled()
                            if let usd = displayUSD {
                                HStack {
                                    Text(String(localized: "label_amount", defaultValue: "Amount"))
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    VStack(alignment: .trailing, spacing: 2) {
                                        Text(usd.usdFormatted)
                                            .fontWeight(.medium)
                                        Text("\(displaySats.btcSpacedFormatted) BTC")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                HStack {
                                    Text(String(localized: "label_network_fee", defaultValue: "Network fee"))
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    Text(String(localized: "label_network_fee", defaultValue: "Network fee"))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            if appState.nodeService.channels.contains(where: \.isChannelReady) {
                                Text(String(localized: "info_splice_out", defaultValue: "Will route via splice-out"))
                                    .font(.caption)
                                    .foregroundStyle(.orange)
                            }
                        case .unknown:
                            Label(
                                String(localized: "label_unrecognized_format", defaultValue: "Unrecognized format"),
                                systemImage: "questionmark.circle"
                            )
                            .foregroundStyle(.secondary)
                        }
                    }

                    if let error = errorMessage {
                        Section {
                            Label(error, systemImage: "exclamationmark.triangle")
                                .foregroundStyle(.red)
                        }
                    }

                    // Send button is below the form as a sticky bar
                }

                if success {
                    Section {
                        VStack(spacing: 4) {
                            Label(
                                String(localized: "success_sent", defaultValue: "Payment sent!"),
                                systemImage: "checkmark.circle.fill"
                            )
                            .foregroundStyle(.green)
                            if sentAmountSats > 0 {
                                let price = appState.btcPrice
                                if price > 0 {
                                    let usd = Double(sentAmountSats) / Double(Constants.satsInBTC) * price
                                    Text(usd.usdFormatted)
                                        .fontWeight(.medium)
                                }
                                Text("\(sentAmountSats.btcSpacedFormatted) BTC")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .safeAreaInset(edge: .bottom) {
                if detectedType != .unknown && !success {
                    Button {
                        Task { await send() }
                    } label: {
                        if isSending {
                            ProgressView()
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 14)
                        } else {
                            Text(String(localized: "button_send_payment", defaultValue: "Send Payment"))
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 14)
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(.blue)
                    .disabled(isSending || success || needsAmount)
                    .padding(.horizontal)
                    .padding(.bottom, 8)
                }
            }
            .navigationTitle(String(localized: "button_send", defaultValue: "Send"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(success ? String(localized: "button_done", defaultValue: "Done") : String(
                        localized: "button_cancel",
                        defaultValue: "Cancel"
                    )) { dismiss() }
                }
            }
            .qrInputToolbar(text: $input, sanitize: QRCodeExtractor.sanitizePaymentInput)
        }
    }

    private var needsAmount: Bool {
        switch detectedType {
        case .bolt11:
            return isAmountlessBolt11 && manualAmountMsat == 0
        case .bolt12, .onchain:
            return displaySats == 0
        default:
            return false
        }
    }

    private func send() async {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        // Dismiss any active keyboard to avoid blocking system auth dialogs
        UIApplication.shared.sendAction(
            Selector(("resignFirstResponder")),
            to: nil,
            from: nil,
            for: nil
        )

        let transactionAuth = UserDefaults.standard.bool(forKey: "transactionAuthEnabled")

        // Auth gate: on-chain always requires auth. Lightning sends require auth only when Payment Confirmation is
        // enabled.
        let requiresAuth: Bool
        let reason: String

        switch detectedType {
        case .onchain:
            requiresAuth = true
            reason = "Confirm onchain withdrawal of all funds"
        case .bolt11, .bolt12:
            requiresAuth = transactionAuth
            reason = "Confirm payment of \(displaySats) sats"
        default:
            requiresAuth = false
            reason = ""
        }

        if requiresAuth {
            let authPassed = await appState.authenticate(reason: reason)
            guard authPassed else {
                errorMessage = appState.authError ?? "Authentication required to send."
                return
            }
        }

        appState.ensureLSPConnected()
        isSending = true
        errorMessage = nil
        defer { isSending = false }

        do {
            let price = appState.btcPrice

            switch detectedType {
            case .bolt11:
                let bolt11 = try Bolt11Invoice.fromStr(invoiceStr: trimmed)
                let invoiceMsat = bolt11.amountMilliSatoshis() ?? 0
                let paymentId: PaymentId
                let actualMsat: UInt64
                if invoiceMsat > 0 {
                    paymentId = try appState.nodeService.sendPayment(invoice: bolt11)
                    actualMsat = invoiceMsat
                } else {
                    actualMsat = manualAmountMsat
                    paymentId = try appState.nodeService.sendPaymentUsingAmount(invoice: bolt11, amountMsat: actualMsat)
                }
                let invoiceUSD: Double? = (price > 0 && actualMsat > 0) ? (
                    Double(actualMsat) / 1000.0 / 100_000_000.0
                ) *
                    price : nil
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: "\(paymentId)",
                    paymentType: "lightning",
                    direction: "sent",
                    amountMsat: actualMsat,
                    amountUSD: invoiceUSD,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending"
                )
                sentAmountSats = actualMsat / 1000

            case .bolt12:
                let sats = displaySats
                guard sats > 0 else { return }
                let offer = try Offer.fromStr(offerStr: trimmed)
                let msat = sats * 1000
                let paymentId = try appState.nodeService.sendBolt12UsingAmount(offer: offer, amountMsat: msat)
                let amountUSD: Double? = price > 0 ? (Double(sats) / Double(Constants.satsInBTC)) * price : nil
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: "\(paymentId)",
                    paymentType: "bolt12",
                    direction: "sent",
                    amountMsat: msat,
                    amountUSD: amountUSD,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending"
                )
                sentAmountSats = sats

            case .onchain:
                let sats = displaySats
                guard sats > 0 else { return }
                let amountUSD: Double? = price > 0 ? (Double(sats) / Double(Constants.satsInBTC)) * price : nil
                // Route through splice-out if channel exists
                if let channel = appState.nodeService.channels.first(where: { $0.isChannelReady }) {
                    guard !appState.isSweeping else {
                        throw NSError(
                            domain: "",
                            code: 0,
                            userInfo: [NSLocalizedDescriptionKey: "A splice is already in progress — try again shortly"]
                        )
                    }
                    try appState.beginSpliceOut(amountSats: sats, address: trimmed)
                    do {
                        try appState.nodeService.spliceOut(
                            userChannelId: channel.userChannelId,
                            counterpartyNodeId: channel.counterpartyNodeId,
                            address: trimmed,
                            amountSats: sats
                        )
                    } catch {
                        appState.cancelPendingSpliceStart()
                        throw error
                    }
                } else {
                    let txid = try appState.nodeService.sendOnchain(address: trimmed, amountSats: sats)
                    _ = try? appState.databaseService?.recordPayment(
                        paymentId: txid,
                        paymentType: "onchain",
                        direction: "sent",
                        amountMsat: sats * 1000,
                        amountUSD: amountUSD,
                        btcPrice: price > 0 ? price : nil,
                        counterparty: nil,
                        status: "pending",
                        txid: txid,
                        address: trimmed
                    )
                }
                sentAmountSats = sats

            case .unknown:
                errorMessage = String(
                    localized: "error_unrecognized_format",
                    defaultValue: "Unrecognized payment format"
                )
                return
            }

            success = true
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
