import SwiftUI

struct OnChainSendView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
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
            Form {
                Section("Destination Address") {
                    TextField("bc1...", text: $address)
                        .font(.system(.body, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }

                Section("Amount") {
                    if sendAll {
                        Text("All available funds")
                            .foregroundStyle(.secondary)
                    } else {
                        HStack {
                            Text("$")
                                .foregroundStyle(.secondary)
                            TextField("0.00", text: $amountUSDStr)
                                .keyboardType(.decimalPad)
                        }
                        if let sats = amountSats, sats > 0 {
                            Text("\(sats.btcSpacedFormatted) BTC")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    Toggle("Send All", isOn: $sendAll)
                }

                if hasReadyChannel {
                    Section {
                        Text("Funds will be sent via splice-out from your Lightning channel.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                Section {
                    Button {
                        Task { await send() }
                    } label: {
                        if isSending {
                            ProgressView().frame(maxWidth: .infinity)
                        } else {
                            Text("Send").frame(maxWidth: .infinity)
                        }
                    }
                    .disabled(address.isEmpty || (!sendAll && (amountSats ?? 0) == 0) || isSending)
                }

                if let txid {
                    Section {
                        Label("Sent!", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        Text("TXID: \(txid)")
                            .font(.caption)
                            .textSelection(.enabled)
                    }
                }

                if spliceSuccess {
                    Section {
                        Label("Splice-out initiated!", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        Text("Funds will arrive on-chain after confirmation.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                if let error = errorMessage {
                    Section {
                        Text(error).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Send On-Chain")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }

    private func send() async {
        isSending = true
        errorMessage = nil
        defer { isSending = false }

        let sats = sendAll ? UInt64(0) : (amountSats ?? 0)
        guard sendAll || sats > 0 else { return }

        do {
            // If channel exists, route through splice-out
            if let channel = appState.nodeService.channels.first(where: { $0.isChannelReady }), !sendAll {
                guard !appState.isSweeping else {
                    throw NSError(domain: "", code: 0, userInfo: [NSLocalizedDescriptionKey: "A splice is already in progress — try again shortly"])
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
