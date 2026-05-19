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
                Section(String(localized: "section_node", defaultValue: "Node")) {
                    HStack {
                        Text(String(localized: "label_status", defaultValue: "Status"))
                        Spacer()
                        HStack(spacing: 4) {
                            Circle()
                                .fill(appState.nodeService.isRunning ? .green : .red)
                                .frame(width: 8, height: 8)
                            Text(appState.nodeService.isRunning
                                ? String(localized: "status_running", defaultValue: "Running")
                                : String(localized: "status_stopped", defaultValue: "Stopped"))
                                .foregroundStyle(appState.nodeService.isRunning ? .green : .red)
                        }
                    }

                    if showNodeId {
                        copyableRow(
                            String(localized: "label_node_id", defaultValue: "Node ID"),
                            appState.nodeService.nodeId
                        )
                    } else {
                        Button(String(localized: "button_show_node_id", defaultValue: "Show Node ID")) {
                            showNodeId = true
                        }
                    }
                }

                // Privacy & Security
                Section(String(localized: "section_privacy_security", defaultValue: "Privacy & Security")) {
                    NavigationLink("App Access") {
                        AppAccessSettingsView()
                    }
                }

                // Channel Info
                if let channel = appState.nodeService.channels.first {
                    Section(String(localized: "section_channel", defaultValue: "Channel")) {
                        HStack {
                            Text(String(localized: "label_capacity", defaultValue: "Capacity"))
                            Spacer()
                            Text(channel.channelValueSats.satsFormatted)
                        }
                        HStack {
                            Text(String(localized: "label_status", defaultValue: "Status"))
                            Spacer()
                            HStack(spacing: 4) {
                                Circle()
                                    .fill(channel.isChannelReady ? .green : .orange)
                                    .frame(width: 8, height: 8)
                                Text(channel.isChannelReady
                                    ? String(localized: "channel_status_ready", defaultValue: "Ready")
                                    : String(localized: "channel_status_pending", defaultValue: "Pending"))
                            }
                        }
                        HStack {
                            Text(String(localized: "label_outbound", defaultValue: "Outbound"))
                            Spacer()
                            Text((channel.outboundCapacityMsat / 1000).satsFormatted)
                        }
                        HStack {
                            Text(String(localized: "label_inbound", defaultValue: "Inbound"))
                            Spacer()
                            Text(((channel.inboundCapacityMsat) / 1000).satsFormatted)
                        }

                        if let txid = appState.fundingTxid, !txid.isEmpty {
                            HStack {
                                Text(String(localized: "label_funding_tx", defaultValue: "Funding Tx"))
                                    .foregroundStyle(.secondary)
                                Spacer()
                                Text(String(txid.prefix(8)) + "..." + String(txid.suffix(8)))
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(.secondary)
                            }
                            if let url = URL(string: "https://mempool.space/tx/\(txid)") {
                                Link(destination: url) {
                                    HStack(spacing: 4) {
                                        Text(String(localized: "view_on_explorer", defaultValue: "View on explorer"))
                                        Image(systemName: "arrow.up.right.square")
                                    }
                                    .font(.caption)
                                    .foregroundStyle(.blue)
                                }
                            }
                        }
                    }

                    Section(String(localized: "section_stable_position", defaultValue: "Stable Position")) {
                        HStack {
                            Text(String(localized: "label_expected_usd", defaultValue: "Expected USD"))
                            Spacer()
                            Text(appState.stableChannel.expectedUSD.formatted)
                                .fontWeight(.medium)
                        }
                        HStack {
                            Text(String(localized: "label_backing_sats", defaultValue: "Backing Sats"))
                            Spacer()
                            Text(appState.stableChannel.backingSats.satsFormatted)
                        }
                        HStack {
                            Text(String(localized: "label_native_btc", defaultValue: "Native BTC"))
                            Spacer()
                            Text(appState.stableChannel.nativeChannelBTC.sats.satsFormatted)
                        }

                        // Stability status
                        if appState.btcPrice > 0 && appState.stableChannel.expectedUSD.amount > 0 {
                            let result = StabilityService.checkStabilityAction(
                                appState.stableChannel, price: appState.btcPrice
                            )
                            HStack {
                                Text(String(localized: "label_status", defaultValue: "Status"))
                                Spacer()
                                Text(result.action.rawValue)
                                    .foregroundStyle(stabilityColor(result.action))
                                    .fontWeight(.medium)
                            }
                            HStack {
                                Text(String(localized: "label_distance_from_par", defaultValue: "Distance from Par"))
                                Spacer()
                                Text(String(format: "%.2f%%", result.percentFromPar))
                                    .foregroundStyle(result.percentFromPar < 0.1 ? .green : .orange)
                            }
                        }

                        if !appState.stableChannel.counterparty.isEmpty {
                            copyableRow(
                                String(localized: "label_counterparty", defaultValue: "Counterparty"),
                                appState.stableChannel.counterparty
                            )
                        }
                    }

                    Section {
                        Button(
                            String(localized: "button_close_channel", defaultValue: "Close Channel"),
                            role: .destructive
                        ) {
                            showCloseChannelAlert = true
                        }
                    }
                }

                // On-Chain
                Section(String(localized: "section_on_chain", defaultValue: "On-Chain")) {
                    HStack {
                        Text(String(localized: "label_balance", defaultValue: "Balance"))
                        Spacer()
                        Text(appState.onchainBalanceSats.satsFormatted)
                    }

                    if appState.onchainBalanceSats > 0 {
                        NavigationLink(String(localized: "link_send_on_chain", defaultValue: "Send On-Chain")) {
                            OnChainSendView()
                        }
                    }
                }

                // Push Notifications
                Section(String(localized: "section_notifications", defaultValue: "Push Notifications")) {
                    HStack {
                        Text(String(localized: "label_notifications", defaultValue: "Notifications"))
                        Spacer()
                        if notificationsEnabled {
                            Text(String(localized: "notifications_enabled", defaultValue: "Enabled"))
                                .foregroundStyle(.green)
                        } else {
                            Text(String(localized: "notifications_disabled", defaultValue: "Disabled"))
                                .foregroundStyle(.red)
                        }
                    }

                    if !notificationsEnabled {
                        Button(String(localized: "button_enable_settings", defaultValue: "Enable in Settings")) {
                            if let url = URL(string: UIApplication.openSettingsURLString) {
                                UIApplication.shared.open(url)
                            }
                        }
                        Text(String(
                            localized: "info_notifications_needed",
                            defaultValue: "Notifications are required to receive stability payments while the app is closed."
                        ))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }

                    if let token = UserDefaults.standard.string(forKey: "apns_device_token") {
                        copyableRow(String(localized: "label_device_token", defaultValue: "Device Token"), token)
                    }
                }

                // Backup
                Section(String(localized: "section_backup", defaultValue: "Backup")) {
                    Button(showSeedWords
                        ? String(localized: "button_hide_seed", defaultValue: "Hide Seed Words")
                        : String(localized: "button_backup_seed", defaultValue: "Backup Seed Words")) {
                            showSeedWords.toggle()
                        }
                    if showSeedWords {
                        if let words = appState.nodeService.savedMnemonic, !words.isEmpty {
                            Text(String(
                                localized: "warning_seed",
                                defaultValue: "Write these words down on paper and store them in a safe place. Never share them. Anyone with these words can access your funds."
                            ))
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
                                    Text(copiedField == "Seed Words"
                                        ? String(localized: "button_copied", defaultValue: "Copied")
                                        : String(localized: "button_copy_seed", defaultValue: "Copy Seed Words"))
                                }
                            }
                        } else {
                            Text(String(
                                localized: "info_seed_unavailable",
                                defaultValue: "Seed phrase not available for this wallet."
                            ))
                            .foregroundStyle(.secondary)
                        }
                    }
                    Button(String(localized: "button_restore_seed", defaultValue: "Restore from Seed")) {
                        showRestore = true
                    }
                }

                // About
                Section(String(localized: "section_about", defaultValue: "About")) {
                    HStack {
                        Text(String(localized: "label_version", defaultValue: "Version"))
                        Spacer()
                        Text(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—")
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Text(String(localized: "label_network", defaultValue: "Network"))
                        Spacer()
                        Text(Constants.defaultNetwork)
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Text(String(localized: "label_custody", defaultValue: "Custody"))
                        Spacer()
                        Text(String(localized: "custody_self", defaultValue: "Self-custodial"))
                            .foregroundStyle(.secondary)
                    }
                    Text(String(
                        localized: "info_self_custody",
                        defaultValue: "Stable Channels is a self-custodial wallet. You control your private keys. No third party can access or freeze your funds."
                    ))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            .navigationTitle(String(localized: "title_settings", defaultValue: "Settings"))
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
                        Text(String(
                            localized: "instruction_restore",
                            defaultValue: "Enter your 12 or 24-word seed phrase to restore a wallet."
                        ))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal)

                        TextField(
                            String(localized: "placeholder_seed", defaultValue: "word1 word2 word3 ..."),
                            text: $restoreMnemonic,
                            axis: .vertical
                        )
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
                                Text(String(localized: "button_restore", defaultValue: "Restore"))
                                    .frame(maxWidth: .infinity)
                            }
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .disabled(isRestoring || restoreMnemonic.trimmingCharacters(in: .whitespaces).isEmpty)
                        .padding(.horizontal, 32)

                        Spacer()
                    }
                    .padding(.top)
                    .navigationTitle(String(localized: "title_restore_seed", defaultValue: "Restore from Seed"))
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button(String(localized: "button_cancel", defaultValue: "Cancel")) {
                                showRestore = false
                                restoreMnemonic = ""
                                restoreError = nil
                            }
                        }
                    }
                }
            }
            .alert(
                String(localized: "alert_close_channel_title", defaultValue: "Close Channel?"),
                isPresented: $showCloseChannelAlert
            ) {
                Button(String(localized: "alert_close_channel_cancel", defaultValue: "Cancel"), role: .cancel) { }
                Button(String(localized: "alert_close_channel_confirm", defaultValue: "Close"), role: .destructive) {
                    closeChannel()
                }
            } message: {
                Text(String(
                    localized: "alert_close_channel_message",
                    defaultValue: "This will cooperatively close the channel and sweep funds on-chain."
                ))
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
                    Label(String(localized: "button_copied", defaultValue: "Copied"), systemImage: "checkmark")
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
            restoreError = String(
                localized: "error_seed_word_count",
                defaultValue: "Seed phrase must be 12 or 24 words"
            )
            return
        }

        // Stop existing node first
        appState.nodeService.stop()

        do {
            try await appState.nodeService.start(
                network: .bitcoin,
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
