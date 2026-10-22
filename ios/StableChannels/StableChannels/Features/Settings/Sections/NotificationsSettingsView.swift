import SwiftUI
import UIKit

struct NotificationsSettingsView: View {
    @Environment(AppState.self) private var appState
    @Binding var notificationsEnabled: Bool

    var body: some View {
        List {
            Section {
                HStack {
                    Text(String(localized: .labelNotifications))
                    Spacer()
                    if notificationsEnabled {
                        Text(String(localized: .notificationsEnabled))
                            .foregroundStyle(.green)
                    } else {
                        Text(String(localized: .notificationsDisabled))
                            .foregroundStyle(.red)
                    }
                }
            } header: {
                Text(String(localized: .labelStatus))
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
                            Text(String(localized: .buttonEnableInSettings))
                            Spacer()
                            Image(systemName: "arrow.up.right")
                                .font(.caption)
                        }
                    }
                } footer: {
                    Text(String(localized: .infoNotificationsNeeded))
                }
            }
        }
        .navigationTitle(String(localized: .titleNotifications))
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
