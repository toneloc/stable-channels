import SwiftUI
import LDKNode

struct SendView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var input = ""
    @State private var amountSats = ""
    @State private var isSending = false
    @State private var errorMessage: String?
    @State private var success = false
    @State private var sentAmountSats: UInt64 = 0

    private enum InputType {
        case bolt11
        case bolt12
        case onchain
        case unknown
    }

    private var detectedType: InputType {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if trimmed.hasPrefix("lnbc") || trimmed.hasPrefix("lntb") || trimmed.hasPrefix("lnts") {
            return .bolt11
        } else if trimmed.hasPrefix("lno") {
            return .bolt12
        } else if trimmed.hasPrefix("bc1") || trimmed.hasPrefix("1") || trimmed.hasPrefix("3") || trimmed.hasPrefix("tb1") {
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

    /// Sats being sent (from invoice or manual entry)
    private var displaySats: UInt64 {
        switch detectedType {
        case .bolt11:
            return (parsedBolt11Msat ?? 0) / 1000
        case .bolt12, .onchain:
            return UInt64(amountSats) ?? 0
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
                Section("Invoice, Offer, or Address") {
                    TextField("Paste invoice, bolt12 offer, or address...", text: $input, axis: .vertical)
                        .font(.system(.body, design: .monospaced))
                        .lineLimit(3...6)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }

                if !input.trimmingCharacters(in: .whitespaces).isEmpty {
                    Section {
                        switch detectedType {
                        case .bolt11:
                            Label("Bolt11 Invoice", systemImage: "bolt.fill")
                                .foregroundStyle(.blue)
                            if let msat = parsedBolt11Msat, msat > 0 {
                                HStack {
                                    Text("Amount")
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    VStack(alignment: .trailing, spacing: 2) {
                                        Text((msat / 1000).satsFormatted)
                                            .fontWeight(.medium)
                                        if let usd = displayUSD {
                                            Text(usd.usdFormatted)
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            } else if parsedBolt11Msat == nil && detectedType == .bolt11 {
                                // No amount in invoice (zero-amount invoice)
                                Text("No amount specified in invoice")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        case .bolt12:
                            Label("Bolt12 Offer", systemImage: "bolt.fill")
                                .foregroundStyle(.purple)
                            TextField("Amount (sats)", text: $amountSats)
                                .keyboardType(.numberPad)
                            if let usd = displayUSD {
                                Text("≈ \(usd.usdFormatted)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        case .onchain:
                            Label("On-chain Address", systemImage: "link")
                                .foregroundStyle(.orange)
                            TextField("Amount (sats)", text: $amountSats)
                                .keyboardType(.numberPad)
                            if let usd = displayUSD {
                                Text("≈ \(usd.usdFormatted)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            if appState.nodeService.channels.contains(where: { $0.isChannelReady }) {
                                Text("Will route via splice-out")
                                    .font(.caption)
                                    .foregroundStyle(.orange)
                            }
                        case .unknown:
                            Label("Unrecognized format", systemImage: "questionmark.circle")
                                .foregroundStyle(.secondary)
                        }
                    }

                    if detectedType != .unknown {
                        Section {
                            Button {
                                Task { await send() }
                            } label: {
                                if isSending {
                                    ProgressView().frame(maxWidth: .infinity)
                                } else {
                                    Text("Send Payment").frame(maxWidth: .infinity)
                                }
                            }
                            .disabled(isSending || success || needsAmount)
                        }
                    }
                }

                if let error = errorMessage {
                    Section {
                        Label(error, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.red)
                    }
                }

                if success {
                    Section {
                        VStack(spacing: 4) {
                            Label("Payment sent!", systemImage: "checkmark.circle.fill")
                                .foregroundStyle(.green)
                            if sentAmountSats > 0 {
                                Text("\(sentAmountSats.satsFormatted)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .navigationTitle("Send")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(success ? "Done" : "Cancel") { dismiss() }
                }
            }
        }
    }

    private var needsAmount: Bool {
        switch detectedType {
        case .bolt12, .onchain:
            return (UInt64(amountSats) ?? 0) == 0
        default:
            return false
        }
    }

    private func send() async {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        isSending = true
        errorMessage = nil
        defer { isSending = false }

        do {
            let price = appState.btcPrice

            switch detectedType {
            case .bolt11:
                let bolt11 = try Bolt11Invoice.fromStr(invoiceStr: trimmed)
                let invoiceMsat = bolt11.amountMilliSatoshis() ?? 0
                let paymentId = try appState.nodeService.sendPayment(invoice: bolt11)
                let invoiceUSD: Double? = (price > 0 && invoiceMsat > 0) ? (Double(invoiceMsat) / 1000.0 / 100_000_000.0) * price : nil
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: "\(paymentId)",
                    paymentType: "lightning",
                    direction: "sent",
                    amountMsat: invoiceMsat,
                    amountUSD: invoiceUSD,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending"
                )
                sentAmountSats = invoiceMsat / 1000

            case .bolt12:
                guard let sats = UInt64(amountSats), sats > 0 else { return }
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
                guard let sats = UInt64(amountSats), sats > 0 else { return }
                let amountUSD: Double? = price > 0 ? (Double(sats) / Double(Constants.satsInBTC)) * price : nil
                // Route through splice-out if channel exists
                if let channel = appState.nodeService.channels.first(where: { $0.isChannelReady }) {
                    try appState.nodeService.spliceOut(
                        userChannelId: channel.userChannelId,
                        counterpartyNodeId: channel.counterpartyNodeId,
                        address: trimmed,
                        amountSats: sats
                    )
                    appState.pendingSplice = PendingSplice(direction: "out", amountSats: sats, address: trimmed)
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
                errorMessage = "Unrecognized payment format"
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
