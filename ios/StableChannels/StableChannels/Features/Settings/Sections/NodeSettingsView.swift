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
                        Text(String(localized: "label_status", defaultValue: "Status"))
                            .font(.subheadline)
                        HStack(spacing: 6) {
                            Circle()
                                .fill(appState.nodeService.isRunning ? .green : .red)
                                .frame(width: 8, height: 8)
                            Text(appState.nodeService.isRunning
                                ? String(localized: "status_running", defaultValue: "Running")
                                : String(localized: "status_stopped", defaultValue: "Stopped"))
                                .font(.caption)
                                .foregroundStyle(appState.nodeService.isRunning ? .green : .red)
                        }
                    }
                    Spacer()
                    Image(systemName: appState.nodeService.isRunning ? "checkmark.circle.fill" : "xmark.circle.fill")
                        .foregroundStyle(appState.nodeService.isRunning ? .green : .red)
                }
            } header: {
                Text(String(localized: "label_node_status", defaultValue: "Node Status"))
            }

            Section {
                if showNodeId {
                    nodeIdRow
                } else {
                    Button {
                        showNodeId = true
                    } label: {
                        HStack {
                            Text(String(localized: "button_show_node_id", defaultValue: "Show Node ID"))
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            } header: {
                Text(String(localized: "label_node_identity", defaultValue: "Node Identity"))
            } footer: {
                Text(String(
                    localized: "info_node_id",
                    defaultValue: "Your node's public key. Share this to receive Lightning payments."
                ))
            }

            Section {
                HStack {
                    Text(String(localized: "label_network", defaultValue: "Network"))
                    Spacer()
                    Text(Constants.defaultNetwork)
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Text(String(localized: "label_explorer", defaultValue: "Explorer"))
                    Spacer()
                    Text(String(appState.chainURL.replacingOccurrences(of: "https://", with: "").prefix(20)))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(String(localized: "label_connection", defaultValue: "Connection"))
            }
        }
        .navigationTitle(String(localized: "title_node", defaultValue: "Node"))
        .navigationBarTitleDisplayMode(.inline)
    }

    private var nodeIdRow: some View {
        Button {
            UIPasteboard.general.string = appState.nodeService.nodeId
            withAnimation { copiedNodeId = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                withAnimation { copiedNodeId = false }
            }
        } label: {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(String(localized: "label_node_id", defaultValue: "Node ID"))
                        .font(.subheadline)
                    Spacer()
                    if copiedNodeId {
                        Label(String(localized: "button_copied", defaultValue: "Copied"), systemImage: "checkmark")
                            .font(.caption)
                            .foregroundStyle(.green)
                            .transition(.scale.combined(with: .opacity))
                    } else {
                        Image(systemName: "doc.on.doc")
                            .font(.caption)
                            .foregroundStyle(Color.stablePrimary)
                    }
                }
                Text(appState.nodeService.nodeId)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
    }
}
