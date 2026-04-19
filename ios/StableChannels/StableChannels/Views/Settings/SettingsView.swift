import SwiftUI
import UserNotifications

struct SettingsView: View {
    @Environment(AppState.self) private var appState
    @State private var showCloseChannelAlert = false
    @State private var showNodeId = false
    @State private var copiedField: String?
    @State private var notificationsEnabled = true
    @State private var showSeedWords = false
    @State private var showRestore = false
    @State private var restoreMnemonic = ""
    @State private var isRestoring = false
    @State private var restoreError: String?

    var body: some View {
        NavigationStack {
            List {
                // Node Info
                Section("Node") {
                    HStack {
                        Text("Status")
                        Spacer()
                        HStack(spacing: 4) {
                            Circle()
                                .fill(appState.nodeService.isRunning ? .green : .red)
                                .frame(width: 8, height: 8)
                            Text(appState.nodeService.isRunning ? "Running" : "Stopped")
                                .foregroundStyle(appState.nodeService.isRunning ? .green : .red)
                        }
                    }

                    if showNodeId {
                        copyableRow("Node ID", appState.nodeService.nodeId)
                    } else {
                        Button("Show Node ID") { showNodeId = true }
                    }
                }

                // Channel Info
                if let channel = appState.nodeService.channels.first {
                    Section("Channel") {
                        HStack {
                            Text("Capacity")
                            Spacer()
                            Text(channel.channelValueSats.satsFormatted)
                        }
                        HStack {
                            Text("Status")
                            Spacer()
                            HStack(spacing: 4) {
                                Circle()
                                    .fill(channel.isChannelReady ? .green : .orange)
                                    .frame(width: 8, height: 8)
                                Text(channel.isChannelReady ? "Ready" : "Pending")
                            }
                        }
                        HStack {
                            Text("Outbound")
                            Spacer()
                            Text((channel.outboundCapacityMsat / 1000).satsFormatted)
                        }
                        HStack {
                            Text("Inbound")
                            Spacer()
                            Text(((channel.inboundCapacityMsat) / 1000).satsFormatted)
                        }

                        if let txid = appState.fundingTxid, !txid.isEmpty {
                            HStack {
                                Text("Funding Tx")
                                    .foregroundStyle(.secondary)
                                Spacer()
                                Text(String(txid.prefix(8)) + "..." + String(txid.suffix(8)))
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(.secondary)
                            }
                            if let url = Constants.explorerTxURL(txid: txid) {
                                Link(destination: url) {
                                    HStack(spacing: 4) {
                                        Text("View on explorer")
                                        Image(systemName: "arrow.up.right.square")
                                    }
                                    .font(.caption)
                                    .foregroundStyle(.blue)
                                }
                            }
                        }
                    }

                    Section("Stable Position") {
                        HStack {
                            Text("Expected USD")
                            Spacer()
                            Text(appState.stableChannel.expectedUSD.formatted)
                                .fontWeight(.medium)
                        }
                        HStack {
                            Text("Backing Sats")
                            Spacer()
                            Text(appState.stableChannel.backingSats.satsFormatted)
                        }
                        HStack {
                            Text("Native BTC")
                            Spacer()
                            Text(appState.stableChannel.nativeChannelBTC.sats.satsFormatted)
                        }

                        // Stability status
                        if appState.btcPrice > 0 && appState.stableChannel.expectedUSD.amount > 0 {
                            let result = StabilityService.checkStabilityAction(
                                appState.stableChannel, price: appState.btcPrice
                            )
                            HStack {
                                Text("Status")
                                Spacer()
                                Text(result.action.rawValue)
                                    .foregroundStyle(stabilityColor(result.action))
                                    .fontWeight(.medium)
                            }
                            HStack {
                                Text("Distance from Par")
                                Spacer()
                                Text(String(format: "%.2f%%", result.percentFromPar))
                                    .foregroundStyle(result.percentFromPar < 0.1 ? .green : .orange)
                            }
                        }

                        if !appState.stableChannel.counterparty.isEmpty {
                            copyableRow("Counterparty", appState.stableChannel.counterparty)
                        }
                    }

                    Section {
                        Button("Close Channel", role: .destructive) {
                            showCloseChannelAlert = true
                        }
                    }
                }

                // On-Chain
                Section("On-Chain") {
                    HStack {
                        Text("Balance")
                        Spacer()
                        Text(appState.onchainBalanceSats.satsFormatted)
                    }

                    if appState.onchainBalanceSats > 0 {
                        NavigationLink("Send On-Chain") {
                            OnChainSendView()
                        }
                    }
                }

                // Push Notifications
                Section("Push Notifications") {
                    HStack {
                        Text("Notifications")
                        Spacer()
                        if notificationsEnabled {
                            Text("Enabled")
                                .foregroundStyle(.green)
                        } else {
                            Text("Disabled")
                                .foregroundStyle(.red)
                        }
                    }

                    if !notificationsEnabled {
                        Button("Enable in Settings") {
                            if let url = URL(string: UIApplication.openSettingsURLString) {
                                UIApplication.shared.open(url)
                            }
                        }
                        Text("Notifications are required to receive stability payments while the app is closed.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    if let token = UserDefaults.standard.string(forKey: "apns_device_token") {
                        copyableRow("Device Token", token)
                    }
                }

                // Backup
                Section("Backup") {
                    Button(showSeedWords ? "Hide Seed Words" : "Backup Seed Words") {
                        showSeedWords.toggle()
                    }
                    if showSeedWords {
                        if let words = appState.nodeService.savedMnemonic, !words.isEmpty {
                            Text("Write these words down on paper and store them in a safe place. Never share them. Anyone with these words can access your funds.")
                                .font(.caption)
                                .foregroundStyle(.orange)
                            ForEach(Array(words.split(separator: " ").enumerated()), id: \.offset) { index, word in
                                HStack {
                                    Text("\(index + 1).")
                                        .foregroundStyle(.secondary)
                                        .frame(width: 30, alignment: .trailing)
                                    Text(String(word))
                                        .font(.system(.body, design: .monospaced))
                                }
                            }
                            Button {
                                UIPasteboard.general.string = words
                                copiedField = "Seed Words"
                            } label: {
                                HStack {
                                    Image(systemName: copiedField == "Seed Words" ? "checkmark" : "doc.on.doc")
                                    Text(copiedField == "Seed Words" ? "Copied" : "Copy Seed Words")
                                }
                            }
                        } else {
                            Text("Seed phrase not available for this wallet.")
                                .foregroundStyle(.secondary)
                        }
                    }
                    Button("Restore from Seed") {
                        showRestore = true
                    }
                }

                // About
                Section("About") {
                    HStack {
                        Text("Version")
                        Spacer()
                        Text(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—")
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Text("Network")
                        Spacer()
                        Text(Constants.defaultNetwork)
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Text("Custody")
                        Spacer()
                        Text("Self-custodial")
                            .foregroundStyle(.secondary)
                    }
                    Text("Stable Channels is a self-custodial wallet. You control your private keys. No third party can access or freeze your funds.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .refreshable {
                appState.refreshBalances()
            }
            .onAppear {
                UNUserNotificationCenter.current().getNotificationSettings { settings in
                    DispatchQueue.main.async {
                        notificationsEnabled = settings.authorizationStatus == .authorized
                    }
                }
            }
            .sheet(isPresented: $showRestore) {
                NavigationStack {
                    VStack(spacing: 20) {
                        Text("Enter your 12 or 24-word seed phrase to restore a wallet.")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal)

                        TextField("word1 word2 word3 ...", text: $restoreMnemonic, axis: .vertical)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .lineLimit(3...5)
                            .font(.system(.body, design: .monospaced))
                            .padding()
                            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
                            .padding(.horizontal)

                        if let error = restoreError {
                            Text(error)
                                .font(.caption)
                                .foregroundStyle(.red)
                                .padding(.horizontal)
                        }

                        Button {
                            Task { await restoreWallet() }
                        } label: {
                            if isRestoring {
                                ProgressView().frame(maxWidth: .infinity)
                            } else {
                                Text("Restore").frame(maxWidth: .infinity)
                            }
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .disabled(isRestoring || restoreMnemonic.trimmingCharacters(in: .whitespaces).isEmpty)
                        .padding(.horizontal, 32)

                        Spacer()
                    }
                    .padding(.top)
                    .navigationTitle("Restore from Seed")
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") {
                                showRestore = false
                                restoreMnemonic = ""
                                restoreError = nil
                            }
                        }
                    }
                }
            }
            .alert("Close Channel?", isPresented: $showCloseChannelAlert) {
                Button("Cancel", role: .cancel) { }
                Button("Close", role: .destructive) {
                    closeChannel()
                }
            } message: {
                Text("This will cooperatively close the channel and sweep funds on-chain.")
            }
        }
    }

    private func copyableRow(_ label: String, _ value: String) -> some View {
        Button {
            UIPasteboard.general.string = value
            copiedField = label
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copiedField = nil }
        } label: {
            HStack {
                Text(label)
                    .foregroundStyle(.primary)
                Spacer()
                if copiedField == label {
                    Label("Copied", systemImage: "checkmark")
                        .font(.caption)
                        .foregroundStyle(.green)
                } else {
                    Text(String(value.prefix(8)) + "..." + String(value.suffix(8)))
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func stabilityColor(_ action: StabilityService.StabilityAction) -> Color {
        switch action {
        case .stable: return .green
        case .pay: return .orange
        case .checkOnly: return .blue
        case .highRiskNoAction: return .red
        }
    }

    private func closeChannel() {
        guard let channel = appState.nodeService.channels.first else { return }
        appState.isChannelClosing = true
        try? appState.nodeService.closeChannel(
            userChannelId: channel.userChannelId,
            counterpartyNodeId: channel.counterpartyNodeId
        )
    }

    private func restoreWallet() async {
        isRestoring = true
        restoreError = nil
        defer { isRestoring = false }

        let input = restoreMnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
        let wordCount = input.split(separator: " ").count
        guard wordCount == 12 || wordCount == 24 else {
            restoreError = "Seed phrase must be 12 or 24 words"
            return
        }

        // Stop existing node first
        appState.nodeService.stop()

        do {
            try await appState.nodeService.start(
                network: appState.runtimeNetwork,
                esploraURL: appState.chainURL,
                mnemonic: input
            )
            await MainActor.run {
                showRestore = false
                restoreMnemonic = ""
                appState.refreshBalances()
            }
        } catch {
            await MainActor.run {
                restoreError = error.localizedDescription
            }
        }
    }
}
