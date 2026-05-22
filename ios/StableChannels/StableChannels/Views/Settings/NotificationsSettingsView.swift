import SwiftUI
import UIKit

struct NotificationsSettingsView: View {
    @Environment(AppState.self) private var appState
    @Binding var notificationsEnabled: Bool

    var body: some View {
        List {
            Section {
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
            } header: {
                Text(String(localized: "label_status", defaultValue: "Status"))
            }

            if !notificationsEnabled {
                Section {
                    Button {
                        if let url = URL(string: UIApplication.openSettingsURLString) {
                            UIApplication.shared.open(url)
                        }
                    } label: {
                        HStack {
                            Image(systemName: "gear")
                            Text(String(localized: "button_enable_in_settings", defaultValue: "Enable in Settings"))
                            Spacer()
                            Image(systemName: "arrow.up.right")
                                .font(.caption)
                        }
                    }
                } footer: {
                    Text(String(
                        localized: "info_notifications_needed",
                        defaultValue: "Notifications are required to receive stability payments while the app is closed."
                    ))
                }
            }

            if let token = UserDefaults.standard.string(forKey: "apns_device_token") {
                Section {
                    Button {
                        UIPasteboard.general.string = token
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(String(localized: "label_device_token", defaultValue: "Device Token"))
                                    .font(.subheadline)
                                Text(String(token.prefix(16)) + "...")
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Image(systemName: "doc.on.doc")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                } header: {
                    Text(String(localized: "label_push_token", defaultValue: "Push Token"))
                } footer: {
                    Text(String(
                        localized: "info_device_token",
                        defaultValue: "Used to deliver push notifications to this device."
                    ))
                }
            }
        }
        .navigationTitle(String(localized: "title_notifications", defaultValue: "Notifications"))
        .navigationBarTitleDisplayMode(.inline)
        .onAppear {
            checkNotificationStatus()
        }
    }

    private func checkNotificationStatus() {
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            DispatchQueue.main.async {
                notificationsEnabled = settings.authorizationStatus == .authorized
            }
        }
    }
}
