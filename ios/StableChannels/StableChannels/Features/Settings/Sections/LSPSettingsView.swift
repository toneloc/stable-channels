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
        appState.hasActiveChannel
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
        .navigationTitle(String(localized: .titleLsp))
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { loadCurrentConfig() }
        .alert(
            String(localized: .alertConnectLspTitle),
            isPresented: $showConnectAlert
        ) {
            Button(String(localized: .alertCancel), role: .cancel) {}
            Button(String(localized: .alertConnect)) {
                applyCustomLSP()
            }
        } message: {
            Text(String(localized: .alertConnectLspMessage))
        }
        .alert(
            String(localized: .alertResetLspTitle),
            isPresented: $showResetAlert
        ) {
            Button(String(localized: .alertCancel), role: .cancel) {}
            Button(String(localized: .alertReset), role: .destructive) {
                resetToDefault()
            }
        } message: {
            Text(String(localized: .alertResetLspMessage))
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
                        Text(String(localized: .badgeDefault))
                            .font(.caption2)
                            .fontWeight(.medium)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(Color.green.opacity(0.12))
                            .foregroundStyle(.green)
                            .clipShape(Capsule())
                    } else {
                        Text(String(localized: .badgeCustom))
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
                    label: String(localized: .labelPubkey),
                    value: String(appState.activeLSP.pubkey.prefix(12)) + "..." +
                        String(appState.activeLSP.pubkey.suffix(8))
                )
                configRow(
                    label: String(localized: .labelAddress),
                    value: appState.activeLSP.address
                )
            }
            .padding(.vertical, 4)
        } header: {
            Text(String(localized: .sectionActiveLsp))
        } footer: {
            if !appState.isOnline {
                Text(String(localized: .infoActiveLspOffline))
            }
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
                    Text(String(localized: .lspLockedTitle))
                        .font(.subheadline)
                        .fontWeight(.medium)
                    Text(String(localized: .lspLockedBody))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.vertical, 4)
        } footer: {
            if !appState.isOnline {
                Text(String(localized: .lspLockedFooterOffline))
            } else {
                Text(String(localized: .lspLockedFooter))
            }
        }
    }

    // MARK: - Custom LSP Form

    private var customLSPFormSection: some View {
        Section {
            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: .labelDisplayName))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: .placeholderAlias),
                    text: $alias
                )
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
            }
            .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: .labelNodePubkey))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: .placeholderPubkey),
                    text: $pubkey
                )
                .font(.system(.caption, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            }
            .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: .labelHostPort))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: .placeholderAddress),
                    text: $address
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
            }
            .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 4) {
                Text(String(localized: .labelLsps2Token))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: .placeholderToken),
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
                    Text(String(localized: .lspSwitchSuccess))
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
                            String(localized: .buttonConnectLsp),
                            systemImage: "bolt.fill"
                        )
                        .fontWeight(.semibold)
                    }
                    Spacer()
                }
            }
            .disabled(!appState.isOnline || isRestarting)
            .padding(.vertical, 4)
        } header: {
            Text(String(localized: .sectionCustomLsp))
        } footer: {
            if !appState.isOnline {
                Text(String(localized: .infoCustomLspOffline))
            } else {
                Text(String(localized: .infoCustomLsp))
            }
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
                    Text(String(localized: .buttonResetLsp))
                }
            }
            .disabled(!appState.isOnline || appState.activeLSP.isDefault || isRestarting)
        }
    }

    // MARK: - Helpers

    private var connectionColor: Color {
        guard appState.isOnline else { return .orange }
        guard appState.nodeService.isRunning else { return .red }
        let isConnected = appState.nodeService.channels.contains {
            $0.counterpartyNodeId == appState.activeLSP.pubkey
        }
        return isConnected || appState.nodeService.isRunning ? .green : .orange
    }

    private var connectionLabel: String {
        guard appState.isOnline else {
            return String(localized: .statusOffline)
        }
        guard appState.nodeService.isRunning else {
            return String(localized: .statusDisconnected)
        }
        return String(localized: .statusConnected)
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
            validationError = String(localized: .errorAliasEmpty)
            return
        }
        guard LSPConfig.isValidPubkey(trimmedPubkey) else {
            validationError = String(localized: .errorPubkeyInvalid)
            return
        }
        guard LSPConfig.isValidAddress(trimmedAddress) else {
            validationError = String(localized: .errorAddressInvalid)
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
                validationError = String(localized: .errorLspRestartFailed)
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
                validationError = String(localized: .errorLspResetFailed)
            }
        }
    }
}
