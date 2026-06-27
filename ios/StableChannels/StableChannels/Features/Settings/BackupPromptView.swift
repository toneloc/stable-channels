import SwiftUI

struct BackupPromptView: View {
    let backupService: any BackupServiceProtocol

    @Environment(\.dismiss) private var dismiss
    @State private var errorMessage: String?
    @State private var isAuthenticating = false
    @State private var showSuccess = false
    @State private var showingOverwriteAlert = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 32) {
                Spacer()

                Image(systemName: "icloud.fill")
                    .font(.system(size: 80))
                    .foregroundStyle(.blue)
                    .shadow(color: .blue.opacity(0.3), radius: 20, x: 0, y: 8)

                Text(String(localized: "icloud_backup_title", defaultValue: "iCloud Seed Backup"))
                    .font(.title.bold())

                Text(String(
                    localized: "icloud_backup_description",
                    defaultValue: "Protect your wallet with encrypted cloud backup."
                ))
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 24)

                VStack(alignment: .leading, spacing: 16) {
                    featureRow(icon: "lock.shield.fill", text: "Encrypted backup to your iCloud")
                    featureRow(icon: "iphone.gen3", text: "Sync across all your devices")
                    featureRow(icon: "key.fill", text: "AES-256 encryption")
                }
                .padding(.horizontal, 32)
                .padding(.top, 8)

                enableBackupWarning

                Spacer()

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

                VStack(spacing: 16) {
                    Button {
                        Task { await handleEnableTap() }
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
                    .buttonStyle(.borderedProminent)
                    .disabled(isAuthenticating)

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
            Spacer()
        }
    }

    private var enableBackupWarning: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text("Important")
                    .font(.headline)
                Spacer()
            }

            Text(
                "This backup contains only your seed phrase. Lightning channel state is NOT included. If you need to recover, you may lose access to any Lightning funds."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            Text("Please withdraw Lightning funds before proceeding if you plan to use this backup for recovery.")
                .font(.caption)
                .foregroundStyle(.orange)
                .fontWeight(.medium)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding()
        .background(.orange.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal, 24)
    }

    private func handleEnableTap() async {
        isAuthenticating = true
        let exists = await backupService.checkRemoteBackupExists()
        isAuthenticating = false
        if exists {
            showingOverwriteAlert = true
        } else {
            await enableBackup()
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
