import SwiftUI

struct NodeSettingsView: View {
    @Environment(AppState.self) private var appState
    @State private var showNodeId = false
    @State private var copiedNodeId = false

    var body: some View {
        List {
            Section {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: .labelStatus))
                            .font(.subheadline)
                        HStack(spacing: 6) {
                            Circle()
                                .fill(statusColor)
                                .frame(width: 8, height: 8)
                            Text(statusText)
                                .font(.caption)
                                .foregroundStyle(statusColor)
                        }
                    }
                    Spacer()
                    Image(systemName: statusIcon)
                        .foregroundStyle(statusColor)
                }
            } header: {
                Text(String(localized: .labelNodeStatus))
            }

            Section {
                if showNodeId {
                    nodeIdRow
                } else {
                    Button {
                        showNodeId = true
                    } label: {
                        HStack {
                            Text(String(localized: .buttonShowNodeId))
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            } header: {
                Text(String(localized: .labelNodeIdentity))
            } footer: {
                Text(String(localized: .infoNodeId))
            }

            Section {
                HStack {
                    Text(String(localized: .labelNetwork))
                    Spacer()
                    Text(Constants.defaultNetwork)
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Text(String(localized: .labelExplorer))
                    Spacer()
                    Text(String(appState.chainURL.replacingOccurrences(of: "https://", with: "").prefix(20)))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(String(localized: .labelConnection))
            }
        }
        .navigationTitle(String(localized: .titleNode))
        .navigationBarTitleDisplayMode(.inline)
    }

    private var isOnline: Bool { appState.isOnline }
    private var isNodeRunning: Bool { appState.nodeService.isRunning }

    private var statusColor: Color {
        if !isOnline {
            return .orange
        }
        return isNodeRunning ? .green : .red
    }

    private var statusText: String {
        if !isOnline {
            return String(localized: .statusOfflinePaused)
        }
        return isNodeRunning
            ? String(localized: .statusRunning)
            : String(localized: .statusStopped)
    }

    private var statusIcon: String {
        if !isOnline {
            return "wifi.slash"
        }
        return isNodeRunning ? "checkmark.circle.fill" : "xmark.circle.fill"
    }

    private var effectiveNodeId: String {
        if !appState.nodeService.nodeId.isEmpty {
            return appState.nodeService.nodeId
        }
        return UserDefaults(suiteName: Constants.appGroupIdentifier)?.string(forKey: "node_id") ?? ""
    }

    private var nodeIdRow: some View {
        Button {
            UIPasteboard.general.string = effectiveNodeId
            withAnimation { copiedNodeId = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                withAnimation { copiedNodeId = false }
            }
        } label: {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(String(localized: .labelNodeId))
                        .font(.subheadline)
                    Spacer()
                    if copiedNodeId {
                        Label(String(localized: .buttonCopied), systemImage: "checkmark")
                            .font(.caption)
                            .foregroundStyle(.green)
                            .transition(.scale.combined(with: .opacity))
                    } else {
                        Image(systemName: "doc.on.doc")
                            .font(.caption)
                            .foregroundStyle(Color.stablePrimary)
                    }
                }
                Text(effectiveNodeId.isEmpty
                    ? String(localized: .statusOfflineUnknown)
                    : effectiveNodeId)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
    }
}
