import SwiftUI
import UserNotifications

struct SettingsView: View {
    @Environment(AppState.self) private var appState
    @AppStorage("user_theme") private var themeSelection: String = "system"
    @State private var notificationsEnabled = true

    var body: some View {
        NavigationStack {
            List {
                // MARK: - Wallet

                Section {
                    NavigationLink {
                        StablePositionView()
                    } label: {
                        rowLabel(
                            icon: "banknote.fill",
                            color: .green,
                            text: String(localized: "link_stable_position", defaultValue: "Stable Position")
                        )
                    }
                    NavigationLink {
                        ChannelSettingsView()
                    } label: {
                        rowLabel(
                            icon: "arrow.left.arrow.right.circle.fill",
                            color: Color.stablePrimary,
                            text: String(localized: "link_channel", defaultValue: "Channel")
                        )
                    }
                    NavigationLink {
                        BackupSettingsView()
                    } label: {
                        rowLabel(
                            icon: "lock.doc.fill",
                            color: .orange,
                            text: String(localized: "link_backup", defaultValue: "Backup")
                        )
                    }
                    if appState.onchainBalanceSats > 0 {
                        NavigationLink {
                            OnChainSendView()
                        } label: {
                            rowLabel(
                                icon: "bitcoinsign.circle.fill",
                                color: Color.stablePrimary,
                                text: String(localized: "link_send_on_chain", defaultValue: "Send On-Chain")
                            )
                        }
                    }
                } header: {
                    Text(String(localized: "section_wallet", defaultValue: "Wallet"))
                        .font(.headline)
                        .foregroundStyle(.green)
                }

                // MARK: - Preferences

                Section {
                    NavigationLink {
                        AppearanceSettingsView(themeSelection: $themeSelection)
                    } label: {
                        rowLabel(
                            icon: "sparkles",
                            color: .purple,
                            text: String(localized: "link_appearance", defaultValue: "Appearance")
                        )
                    }
                    NavigationLink {
                        NotificationsSettingsView(notificationsEnabled: $notificationsEnabled)
                    } label: {
                        rowLabel(
                            icon: "bell.and.waves.left.and.right.fill",
                            color: .red,
                            text: String(localized: "link_notifications", defaultValue: "Notifications")
                        )
                    }
                } header: {
                    Text(String(localized: "section_preferences", defaultValue: "Preferences"))
                        .font(.headline)
                        .foregroundStyle(.purple)
                }

                // MARK: - Node & Network

                Section {
                    NavigationLink {
                        NodeSettingsView()
                    } label: {
                        rowLabel(
                            icon: "cube.transparent.fill",
                            color: Color.stablePrimary,
                            text: String(localized: "link_node", defaultValue: "Node")
                        )
                    }
                    NavigationLink {
                        PushConnectivitySettingsView()
                    } label: {
                        rowLabel(
                            icon: "wifi.router.fill",
                            color: .green,
                            text: String(localized: "link_push_connectivity", defaultValue: "Push Connectivity")
                        )
                    }
                } header: {
                    Text(String(localized: "section_node_network", defaultValue: "Node & Network"))
                        .font(.headline)
                        .foregroundStyle(Color.stablePrimary)
                }

                // MARK: - Privacy & Security

                Section {
                    NavigationLink {
                        AppAccessSettingsView()
                    } label: {
                        rowLabel(
                            icon: "faceid",
                            color: .indigo,
                            text: String(localized: "link_app_access", defaultValue: "App Access")
                        )
                    }
                } header: {
                    Text(String(localized: "section_privacy_security", defaultValue: "Privacy & Security"))
                        .font(.headline)
                        .foregroundStyle(.indigo)
                }

                // MARK: - About

                Section {
                    NavigationLink {
                        AboutSettingsView()
                    } label: {
                        rowLabel(
                            icon: "questionmark.circle.fill",
                            color: .secondary,
                            text: String(localized: "link_about", defaultValue: "About")
                        )
                    }
                } header: {
                    Text(String(localized: "section_about", defaultValue: "About"))
                        .font(.headline)
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
        }
    }

    private func rowLabel(icon: String, color: Color, text: String) -> some View {
        HStack(spacing: 14) {
            ZStack {
                RoundedRectangle(cornerRadius: 8)
                    .fill(color.opacity(0.12))
                    .frame(width: 30, height: 30)
                Image(systemName: icon)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(color)
            }
            Text(text)
                .font(.body)
                .foregroundStyle(.primary)
        }
        .padding(.vertical, 2)
    }
}
