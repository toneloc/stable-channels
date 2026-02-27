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
                        case .bolt12:
                            Label("Bolt12 Offer", systemImage: "bolt.fill")
                                .foregroundStyle(.purple)
                            TextField("Amount (sats)", text: $amountSats)
                                .keyboardType(.numberPad)
                        case .onchain:
                            Label("On-chain Address", systemImage: "link")
                                .foregroundStyle(.orange)
                            TextField("Amount (sats)", text: $amountSats)
                                .keyboardType(.numberPad)
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
                        Label("Payment sent!", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
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
            switch detectedType {
            case .bolt11:
                let bolt11 = try Bolt11Invoice.fromStr(invoiceStr: trimmed)
                let paymentId = try appState.nodeService.sendPayment(invoice: bolt11)
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: "\(paymentId)",
                    paymentType: "lightning",
                    direction: "sent",
                    amountMsat: 0,
                    amountUSD: nil,
                    btcPrice: appState.btcPrice > 0 ? appState.btcPrice : nil,
                    counterparty: nil,
                    status: "pending"
                )

            case .bolt12:
                guard let sats = UInt64(amountSats), sats > 0 else { return }
                let offer = try Offer.fromStr(offerStr: trimmed)
                let paymentId = try appState.nodeService.sendBolt12UsingAmount(offer: offer, amountMsat: sats * 1000)
                _ = try? appState.databaseService?.recordPayment(
                    paymentId: "\(paymentId)",
                    paymentType: "bolt12",
                    direction: "sent",
                    amountMsat: sats * 1000,
                    amountUSD: nil,
                    btcPrice: appState.btcPrice > 0 ? appState.btcPrice : nil,
                    counterparty: nil,
                    status: "pending"
                )

            case .onchain:
                guard let sats = UInt64(amountSats), sats > 0 else { return }
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
                        amountUSD: nil,
                        btcPrice: appState.btcPrice > 0 ? appState.btcPrice : nil,
                        counterparty: nil,
                        status: "pending",
                        txid: txid,
                        address: trimmed
                    )
                }

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
