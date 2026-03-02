import SwiftUI

struct OnChainSendView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var address = ""
    @State private var amountSats = ""
    @State private var sendAll = false
    @State private var isSending = false
    @State private var errorMessage: String?
    @State private var txid: String?
    @State private var spliceSuccess = false

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

                Section("Amount (sats)") {
                    if sendAll {
                        Text("All available funds")
                            .foregroundStyle(.secondary)
                    } else {
                        TextField("0", text: $amountSats)
                            .keyboardType(.numberPad)
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
                    .disabled(address.isEmpty || (!sendAll && amountSats.isEmpty) || isSending)
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

        let sats = sendAll ? 0 : (UInt64(amountSats) ?? 0)
        guard sendAll || sats > 0 else { return }

        do {
            // If channel exists, route through splice-out
            if let channel = appState.nodeService.channels.first(where: { $0.isChannelReady }), !sendAll {
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
            } else {
                let result = try appState.nodeService.sendOnchain(address: address, amountSats: sats)
                txid = result
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
