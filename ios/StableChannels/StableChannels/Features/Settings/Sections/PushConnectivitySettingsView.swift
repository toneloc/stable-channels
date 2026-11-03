import SwiftUI

struct PushConnectivitySettingsView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        List {
            Section {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: .labelNodeStatus))
                            .font(.subheadline)
                        Text(String(localized: .infoPushDescription))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if !appState.isOnline {
                        Label(String(localized: .statusOffline), systemImage: "bolt.slash")
                            .font(.caption)
                            .foregroundStyle(.orange)
                    } else if appState.nodeService.isRunning {
                        Label(String(localized: .statusActive), systemImage: "bolt.fill")
                            .font(.caption)
                            .foregroundStyle(.green)
                    } else {
                        Label(String(localized: .statusInactive), systemImage: "bolt.slash")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Text(String(localized: .labelBackgroundService))
            }

            Section {
                HStack {
                    Text(String(localized: .labelCounterparty))
                    Spacer()
                    if !appState.stableChannel.counterparty.isEmpty {
                        Text(String(appState.stableChannel.counterparty.prefix(8)) + "...")
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                    } else {
                        Text(String(localized: .labelNone))
                            .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Text(String(localized: .labelChannelInfo))
            } footer: {
                if !appState.isOnline {
                    Text(String(localized: .infoPushConnectivityOffline))
                } else {
                    Text(String(localized: .infoPushConnectivity))
                }
            }
        }
        .navigationTitle(String(localized: .titlePushConnectivity))
        .navigationBarTitleDisplayMode(.inline)
    }
}
