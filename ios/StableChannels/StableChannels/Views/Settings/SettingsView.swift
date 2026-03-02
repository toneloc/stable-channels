import SwiftUI
import UserNotifications

struct SettingsView: View {
    @Environment(AppState.self) private var appState
    @State private var showCloseChannelAlert = false
    @State private var showNodeId = false
    @State private var copiedField: String?
    @State private var notificationsEnabled = true

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

                    NavigationLink("Receive On-Chain") {
                        FundWalletView()
                    }

                    if appState.onchainBalanceSats > 0 {
                        NavigationLink("Send On-Chain") {
                            OnChainSendView()
                        }
                    }
                }

                // Sweep
                if appState.onchainBalanceSats > 0,
                   appState.nodeService.channels.contains(where: { $0.isChannelReady }) {
                    Section("On-Chain Balance") {
                        HStack {
                            Text("Pending Sweep")
                            Spacer()
                            Text(appState.onchainBalanceSats.satsFormatted)
                                .foregroundStyle(.secondary)
                        }
                        Button("Sweep to Channel Now") {
                            sweepToChannel()
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

                // About
                Section("About") {
                    HStack {
                        Text("Version")
                        Spacer()
                        Text("1.0")
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Text("Network")
                        Spacer()
                        Text(Constants.defaultNetwork)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("Settings")
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
        try? appState.nodeService.closeChannel(
            userChannelId: channel.userChannelId,
            counterpartyNodeId: channel.counterpartyNodeId
        )
    }

    private func sweepToChannel() {
        guard let channel = appState.nodeService.channels.first(where: { $0.isChannelReady }) else { return }
        let spendable = appState.nodeService.spendableOnchainSats()
        let feeReserve: UInt64 = 2 * 170  // conservative 2 sat/vB * 170 vB
        guard spendable > feeReserve else { return }
        let sweepAmount = spendable - feeReserve
        do {
            try appState.nodeService.spliceIn(
                userChannelId: channel.userChannelId,
                counterpartyNodeId: channel.counterpartyNodeId,
                amountSats: sweepAmount
            )
            appState.statusMessage = "Sweep initiated (\(sweepAmount) sats)"
        } catch {
            appState.statusMessage = "Sweep failed: \(error.localizedDescription)"
        }
    }
}
