import SwiftUI

struct PushConnectivitySettingsView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        List {
            Section {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: "label_node_status", defaultValue: "Node Status"))
                            .font(.subheadline)
                        Text(String(
                            localized: "info_push_description",
                            defaultValue: "Background payments and stability updates"
                        ))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if !appState.isOnline {
                        Label(String(localized: "status_offline", defaultValue: "Offline"), systemImage: "bolt.slash")
                            .font(.caption)
                            .foregroundStyle(.orange)
                    } else if appState.nodeService.isRunning {
                        Label(String(localized: "status_active", defaultValue: "Active"), systemImage: "bolt.fill")
                            .font(.caption)
                            .foregroundStyle(.green)
                    } else {
                        Label(String(localized: "status_inactive", defaultValue: "Inactive"), systemImage: "bolt.slash")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Text(String(localized: "label_background_service", defaultValue: "Background Service"))
            }

            Section {
                HStack {
                    Text(String(localized: "label_counterparty", defaultValue: "Counterparty"))
                    Spacer()
                    if !appState.stableChannel.counterparty.isEmpty {
                        Text(String(appState.stableChannel.counterparty.prefix(8)) + "...")
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                    } else {
                        Text(String(localized: "label_none", defaultValue: "None"))
                            .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Text(String(localized: "label_channel_info", defaultValue: "Channel Info"))
            } footer: {
                if !appState.isOnline {
                    Text(String(
                        localized: "info_push_connectivity_offline",
                        defaultValue: "Background connection paused until an internet connection is restored."
                    ))
                } else {
                    Text(String(
                        localized: "info_push_connectivity",
                        defaultValue: "Your device maintains a persistent connection to receive incoming payments and stability updates."
                    ))
                }
            }
        }
        .navigationTitle(String(localized: "title_push_connectivity", defaultValue: "Push Connectivity"))
        .navigationBarTitleDisplayMode(.inline)
    }
}
