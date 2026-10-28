import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

struct SettingsView: View {
    @Environment(AppState.self) private var appState
    @AppStorage("user_theme") private var themeSelection: String = "system"
    @State private var notificationsEnabled = true

    var body: some View {
        NavigationStack {
            List {
                // MARK: - Custody Disclaimer

                SettingsBanner(
                    icon: "shield.lefthalf.filled.badge.checkmark",
                    iconColor: .green,
                    title: String(localized: .disclaimerCustodyTitle),
                    bodyText: String(localized: .disclaimerCustodyBody),
                    borderColor: .green
                )

                // MARK: - Wallet

                Section {
                    NavigationLink {
                        StablePositionView()
                    } label: {
                        rowLabel(
                            icon: "banknote.fill",
                            color: .green,
                            text: String(localized: .linkStablePosition)
                        )
                    }
                    NavigationLink {
                        ChannelSettingsView()
                    } label: {
                        rowLabel(
                            icon: "arrow.left.arrow.right.circle.fill",
                            color: Color.stablePrimary,
                            text: String(localized: .linkChannel)
                        )
                    }
                    NavigationLink {
                        BackupSettingsView(backupService: CloudBackupService.shared)
                    } label: {
                        rowLabel(
                            icon: "lock.doc.fill",
                            color: .orange,
                            text: String(localized: .linkBackup)
                        )
                    }
                    if appState.onchainBalanceSats > 0 {
                        NavigationLink {
                            OnChainSendView()
                        } label: {
                            rowLabel(
                                icon: "bitcoinsign.circle.fill",
                                color: Color.stablePrimary,
                                text: String(localized: .linkSendOnChain)
                            )
                        }
                    }
                } header: {
                    Text(String(localized: .sectionWallet))
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
                            text: String(localized: .linkAppearance)
                        )
                    }
                    NavigationLink {
                        NotificationsSettingsView(notificationsEnabled: $notificationsEnabled)
                    } label: {
                        rowLabel(
                            icon: "bell.and.waves.left.and.right.fill",
                            color: .red,
                            text: String(localized: .linkNotifications)
                        )
                    }
                } header: {
                    Text(String(localized: .sectionPreferences))
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
                            text: String(localized: .linkNode)
                        )
                    }
                    NavigationLink {
                        PushConnectivitySettingsView()
                    } label: {
                        rowLabel(
                            icon: "wifi.router.fill",
                            color: .green,
                            text: String(localized: .linkPushConnectivity)
                        )
                    }
                    NavigationLink {
                        LSPSettingsView()
                    } label: {
                        rowLabel(
                            icon: "server.rack",
                            color: .cyan,
                            text: String(localized: .linkLsp)
                        )
                    }
                } header: {
                    Text(String(localized: .sectionNodeNetwork))
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
                            text: String(localized: .linkAppAccess)
                        )
                    }
                } header: {
                    Text(String(localized: .sectionPrivacySecurity))
                        .font(.headline)
                        .foregroundStyle(.indigo)
                }

                // MARK: - Support

                Section {
                    NavigationLink(destination: DiagnosticsSettingsView()) {
                        rowLabel(
                            icon: "stethoscope",
                            color: .green,
                            text: String(localized: .titleLogsDiagnostics)
                        )
                    }
                } header: {
                    Text(String(localized: .headerSupport))
                        .font(.headline)
                        .foregroundStyle(.primary)
                }

                // MARK: - About

                Section {
                    NavigationLink {
                        AboutSettingsView()
                    } label: {
                        rowLabel(
                            icon: "questionmark.circle.fill",
                            color: .secondary,
                            text: String(localized: .linkAbout)
                        )
                    }
                    Button {
                        if let url = URL(string: Constants.privacyPolicyURL) {
                            UIApplication.shared.open(url)
                        }
                    } label: {
                        rowLabel(
                            icon: "hand.raised.fill",
                            color: .secondary,
                            text: String(localized: .linkPrivacyPolicy)
                        )
                    }
                } header: {
                    Text(String(localized: .sectionAbout))
                        .font(.headline)
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle(String(localized: .titleSettings))
            .navigationBarTitleDisplayMode(.inline)
            .refreshable {
                await appState.userInitiatedRefresh()
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
