import SwiftUI

struct LSPSettingsView: View {
    @Environment(AppState.self) private var appState

    @State private var alias = ""
    @State private var pubkey = ""
    @State private var address = ""
    @State private var token = ""

    @State private var showResetAlert = false
    @State private var showConnectAlert = false
    @State private var validationError: String?
    @State private var isRestarting = false
    @State private var showSuccess = false

    private var hasActiveChannel: Bool {
        !appState.nodeService.channels.isEmpty
    }

    var body: some View {
        List {
            activeLSPSection

            if hasActiveChannel {
                channelLockedSection
            } else {
                customLSPFormSection
                resetSection
            }
        }
        .navigationTitle(String(localized: "title_lsp", defaultValue: "Lightning Service Provider"))
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { loadCurrentConfig() }
        .alert(
            String(localized: "alert_connect_lsp_title", defaultValue: "Connect to LSP"),
            isPresented: $showConnectAlert
        ) {
            Button(String(localized: "alert_cancel", defaultValue: "Cancel"), role: .cancel) {}
            Button(String(localized: "alert_connect", defaultValue: "Connect")) {
                applyCustomLSP()
            }
        } message: {
            Text(String(
                localized: "alert_connect_lsp_message",
                defaultValue: "This will restart your node to connect to the new LSP. No funds will be lost."
            ))
        }
        .alert(
            String(localized: "alert_reset_lsp_title", defaultValue: "Reset to Default"),
            isPresented: $showResetAlert
        ) {
            Button(String(localized: "alert_cancel", defaultValue: "Cancel"), role: .cancel) {}
            Button(String(localized: "alert_reset", defaultValue: "Reset"), role: .destructive) {
                resetToDefault()
            }
        } message: {
            Text(String(
                localized: "alert_reset_lsp_message",
                defaultValue: "This will reconnect to the default Stable Channels LSP and restart your node."
            ))
        }
    }

    // MARK: - Active LSP

    private var activeLSPSection: some View {
        Section {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 10) {
                    ZStack {
                        Circle()
                            .fill(connectionColor.opacity(0.15))
                            .frame(width: 36, height: 36)
                        Image(systemName: "server.rack")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(connectionColor)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(appState.activeLSP.alias)
                            .font(.subheadline)
                            .fontWeight(.semibold)
                        HStack(spacing: 4) {
                            Circle()
                                .fill(connectionColor)
                                .frame(width: 6, height: 6)
                            Text(connectionLabel)
                                .font(.caption2)
                                .foregroundStyle(connectionColor)
                        }
                    }
                    Spacer()
                    if appState.activeLSP.isDefault {
                        Text(String(localized: "badge_default", defaultValue: "Default"))
                            .font(.caption2)
                            .fontWeight(.medium)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(Color.green.opacity(0.12))
                            .foregroundStyle(.green)
                            .clipShape(Capsule())
                    } else {
                        Text(String(localized: "badge_custom", defaultValue: "Custom"))
                            .font(.caption2)
                            .fontWeight(.medium)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(Color.cyan.opacity(0.12))
                            .foregroundStyle(.cyan)
                            .clipShape(Capsule())
                    }
                }

                Divider()

                configRow(
                    label: String(localized: "label_pubkey", defaultValue: "Pubkey"),
                    value: String(appState.activeLSP.pubkey.prefix(12)) + "..." +
                        String(appState.activeLSP.pubkey.suffix(8))
                )
                configRow(
                    label: String(localized: "label_address", defaultValue: "Address"),
                    value: appState.activeLSP.address
                )
            }
            .padding(.vertical, 4)
        } header: {
            Text(String(localized: "section_active_lsp", defaultValue: "Active LSP"))
        }
    }

    // MARK: - Channel Locked

    private var channelLockedSection: some View {
        Section {
            HStack(spacing: 12) {
                Image(systemName: "lock.fill")
                    .font(.system(size: 14))
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(String(
                        localized: "lsp_locked_title",
                        defaultValue: "LSP configuration locked"
                    ))
                    .font(.subheadline)
                    .fontWeight(.medium)
                    Text(String(
                        localized: "lsp_locked_body",
                        defaultValue: "Close your active channel to switch LSP."
                    ))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(.vertical, 4)
        } footer: {
            Text(String(
                localized: "lsp_locked_footer",
                defaultValue: "The Lightning Service Provider cannot be changed while a channel is open. The counterparty node is part of the channel's funding transaction."
            ))
        }
    }

    // MARK: - Custom LSP Form

    private var customLSPFormSection: some View {
        Section {
            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: "label_display_name", defaultValue: "Display Name"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: "placeholder_alias", defaultValue: "My LSP"),
                    text: $alias
                )
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
            }
            .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: "label_node_pubkey", defaultValue: "Node Pubkey"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: "placeholder_pubkey", defaultValue: "02... or 03..."),
                    text: $pubkey
                )
                .font(.system(.caption, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            }
            .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: "label_host_port", defaultValue: "Host:Port"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: "placeholder_address", defaultValue: "example.com:9735"),
                    text: $address
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
            }
            .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: "label_lsps2_token", defaultValue: "LSPS2 Token (optional)"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: "placeholder_token", defaultValue: "Leave empty if not required"),
                    text: $token
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            }
            .padding(.vertical, 2)

            if let error = validationError {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.red)
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }

            if showSuccess {
                HStack(spacing: 6) {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                    Text(String(localized: "lsp_switch_success", defaultValue: "Node restarted with new LSP"))
                        .font(.caption)
                        .foregroundStyle(.green)
                }
                .transition(.scale.combined(with: .opacity))
            }

            Button {
                validateAndConnect()
            } label: {
                HStack {
                    Spacer()
                    if isRestarting {
                        ProgressView()
                            .controlSize(.small)
                    } else {
                        Label(
                            String(localized: "button_connect_lsp", defaultValue: "Connect"),
                            systemImage: "bolt.fill"
                        )
                        .fontWeight(.semibold)
                    }
                    Spacer()
                }
            }
            .disabled(isRestarting)
            .padding(.vertical, 4)
        } header: {
            Text(String(localized: "section_custom_lsp", defaultValue: "Connect to a Custom LSP"))
        } footer: {
            Text(String(
                localized: "info_custom_lsp",
                defaultValue: "Enter the details of any LSPS2-compatible Lightning Service Provider. Your node will restart to apply the new configuration."
            ))
        }
    }

    // MARK: - Reset

    private var resetSection: some View {
        Section {
            Button(role: .destructive) {
                showResetAlert = true
            } label: {
                HStack {
                    Image(systemName: "arrow.counterclockwise")
                    Text(String(localized: "button_reset_lsp", defaultValue: "Reset to Default LSP"))
                }
            }
            .disabled(appState.activeLSP.isDefault || isRestarting)
        }
    }

    // MARK: - Helpers

    private var connectionColor: Color {
        guard appState.nodeService.isRunning else { return .red }
        let isConnected = appState.nodeService.channels.contains {
            $0.counterpartyNodeId == appState.activeLSP.pubkey
        }
        // If the node is running we consider it "connected" to the LSP for
        // visual purposes. Peer-level connectivity is managed by LDK internally.
        return isConnected || appState.nodeService.isRunning ? .green : .orange
    }

    private var connectionLabel: String {
        guard appState.nodeService.isRunning else {
            return String(localized: "status_offline", defaultValue: "Offline")
        }
        return String(localized: "status_connected", defaultValue: "Connected")
    }

    private func configRow(label: String, value: String) -> some View {
        HStack {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.secondary)
        }
    }

    private func loadCurrentConfig() {
        let config = appState.activeLSP
        alias = config.alias
        pubkey = config.pubkey
        address = config.address
        token = config.token ?? ""
    }

    private func validateAndConnect() {
        validationError = nil

        let trimmedAlias = alias.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedPubkey = pubkey.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedAddress = address.trimmingCharacters(in: .whitespacesAndNewlines)

        guard !trimmedAlias.isEmpty else {
            validationError = String(localized: "error_alias_empty", defaultValue: "Display name is required.")
            return
        }
        guard LSPConfig.isValidPubkey(trimmedPubkey) else {
            validationError = String(
                localized: "error_pubkey_invalid",
                defaultValue: "Pubkey must be a 66-character hex string starting with 02 or 03."
            )
            return
        }
        guard LSPConfig.isValidAddress(trimmedAddress) else {
            validationError = String(
                localized: "error_address_invalid",
                defaultValue: "Address must be in host:port format (e.g. example.com:9735)."
            )
            return
        }

        showConnectAlert = true
    }

    private func applyCustomLSP() {
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let config = LSPConfig(
            alias: alias.trimmingCharacters(in: .whitespacesAndNewlines),
            pubkey: pubkey.trimmingCharacters(in: .whitespacesAndNewlines),
            address: address.trimmingCharacters(in: .whitespacesAndNewlines),
            token: trimmedToken.isEmpty ? nil : trimmedToken
        )

        isRestarting = true
        validationError = nil

        Task { @MainActor in
            let success = await appState.switchLSP(to: config)
            isRestarting = false
            if success {
                withAnimation { showSuccess = true }
                DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                    withAnimation { showSuccess = false }
                }
            } else {
                validationError = String(
                    localized: "error_lsp_restart_failed",
                    defaultValue: "Failed to connect. The node has been restored to the previous LSP."
                )
            }
        }
    }

    private func resetToDefault() {
        isRestarting = true
        validationError = nil

        Task { @MainActor in
            let success = await appState.switchLSP(to: .default)
            isRestarting = false
            loadCurrentConfig()
            if !success {
                validationError = String(
                    localized: "error_lsp_reset_failed",
                    defaultValue: "Failed to reset. Please try again."
                )
            }
        }
    }
}
