import SwiftUI

struct BackupPromptView: View {
    let backupService: any BackupServiceProtocol

    @Environment(\.dismiss) private var dismiss
    @State private var errorMessage: String?
    @State private var isAuthenticating = false
    @State private var showSuccess = false
    @State private var showingOverwriteAlert = false

    @ViewBuilder
    private var prominentButton: some View {
        if #available(iOS 26.0, *) {
            Button {
                Task {
                    isAuthenticating = true
                    let exists = await backupService.checkRemoteBackupExists()
                    isAuthenticating = false
                    if exists {
                        showingOverwriteAlert = true
                    } else {
                        await enableBackup()
                    }
                }
            } label: {
                HStack(spacing: 8) {
                    if isAuthenticating {
                        ProgressView()
                    } else {
                        Image(systemName: "icloud.and.arrow.up")
                    }
                    Text(String(localized: "enable_icloud_backup", defaultValue: "Enable iCloud Backup"))
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
            }
            .buttonStyle(.glassProminent)
            .disabled(isAuthenticating)
        } else {
            Button {
                Task {
                    isAuthenticating = true
                    let exists = await backupService.checkRemoteBackupExists()
                    isAuthenticating = false
                    if exists {
                        showingOverwriteAlert = true
                    } else {
                        await enableBackup()
                    }
                }
            } label: {
                HStack(spacing: 8) {
                    if isAuthenticating {
                        ProgressView()
                    } else {
                        Image(systemName: "icloud.and.arrow.up")
                    }
                    Text(String(localized: "enable_icloud_backup", defaultValue: "Enable iCloud Backup"))
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .background(.blue)
                .foregroundStyle(.white)
                .clipShape(.rect(cornerRadius: 12))
            }
            .disabled(isAuthenticating)
        }
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 32) {
                Spacer()

                // Icon with soft glow
                Image(systemName: "icloud.fill")
                    .font(.system(size: 80))
                    .foregroundStyle(.blue)
                    .shadow(color: .blue.opacity(0.3), radius: 20, x: 0, y: 8)

                // Title
                Text(String(localized: "icloud_backup_title", defaultValue: "iCloud Seed Backup"))
                    .font(.title.bold())
                    .foregroundStyle(.primary)

                // Subtitle
                Text(String(
                    localized: "icloud_backup_description",
                    defaultValue: "Protect your wallet with encrypted cloud backup."
                ))
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 24)

                // Feature list
                VStack(alignment: .leading, spacing: 16) {
                    featureRow(icon: "lock.shield.fill", text: "Encrypted backup to your iCloud")
                    featureRow(icon: "iphone.gen3", text: "Sync across all your devices")
                    featureRow(icon: "key.fill", text: "AES-256 encryption")
                }
                .padding(.horizontal, 32)
                .padding(.top, 8)

                Spacer()

                // Success state
                if showSuccess {
                    VStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 48))
                            .foregroundStyle(.green)
                        Text("Backup enabled!")
                            .font(.headline)
                            .foregroundStyle(.green)
                    }
                    .padding(.vertical, 16)
                }

                // Buttons
                VStack(spacing: 16) {
                    prominentButton

                    if let error = errorMessage {
                        Text(error)
                            .font(.caption)
                            .foregroundStyle(.red)
                    }

                    Button {
                        dismiss()
                        UserDefaults.standard.set(true, forKey: "backupPromptDismissed")
                    } label: {
                        Text(String(localized: "maybe_later", defaultValue: "Maybe Later"))
                            .font(.subheadline)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.bottom, 24)
            }
            .navigationBarTitleDisplayMode(.inline)
            .interactiveDismissDisabled()
        }
        .nativeTimerAlert(isPresented: $showingOverwriteAlert, title: "Overwrite Existing Backup?") {
            await enableBackup()
        }
    }

    private func featureRow(icon: String, text: String) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .font(.body)
                .foregroundStyle(.blue)
                .frame(width: 24)
            Text(text)
                .font(.subheadline)
                .foregroundStyle(.primary)
            Spacer()
        }
    }

    private func enableBackup() async {
        isAuthenticating = true
        errorMessage = nil

        await backupService.checkAccountStatus()

        if !backupService.iCloudAvailable {
            errorMessage = "Sign in to iCloud to enable backup"
            isAuthenticating = false
            return
        }

        do {
            try await backupService.generateAndStoreKey()
            showSuccess = true
            UserDefaults.standard.set(true, forKey: "backupEnabled")
            UserDefaults.standard.set(true, forKey: "backupPromptDismissed")
            try await Task.sleep(nanoseconds: 1_500_000_000)
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
            isAuthenticating = false
        }
    }
}
