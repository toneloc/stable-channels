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
