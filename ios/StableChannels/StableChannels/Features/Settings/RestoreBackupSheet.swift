import SwiftUI

struct RestoreBackupSheet: View {
    let backupService: any BackupServiceProtocol
    let onRestore: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var isRestoring = false
    @State private var showSuccess = false
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 32) {
                Spacer()

                Image(systemName: "icloud.and.arrow.down")
                    .font(.system(size: 80))
                    .foregroundStyle(.blue)
                    .shadow(color: .blue.opacity(0.3), radius: 20, x: 0, y: 8)

                Text(String(localized: "restore_from_icloud", defaultValue: "Restore from iCloud"))
                    .font(.title.bold())

                Text(String(
                    localized: "restore_description",
                    defaultValue: "Restore your wallet from backup."
                ))
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 24)

                VStack(alignment: .leading, spacing: 16) {
                    featureRow(icon: "key.fill", text: "AES-256 encrypted backup")
                    featureRow(icon: "arrow.clockwise", text: "Sync latest version")
                    featureRow(icon: "checkmark.shield.fill", text: "Verified restore")
                }
                .padding(.horizontal, 32)
                .padding(.top, 8)

                Spacer()

                if showSuccess {
                    VStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 48))
                            .foregroundStyle(.green)
                        Text("Wallet restored!")
                            .font(.headline)
                            .foregroundStyle(.green)
                    }
                    .padding(.vertical, 16)
                } else {
                    restoreButton
                }

                if let error = errorMessage {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .padding(.horizontal, 24)
                }

                Spacer()
            }
            .padding()
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
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

    @ViewBuilder
    private var restoreButton: some View {
        if #available(iOS 26.0, *) {
            Button {
                Task { await performRestore() }
            } label: {
                HStack(spacing: 8) {
                    if isRestoring {
                        ProgressView()
                    } else {
                        Image(systemName: "arrow.down.circle")
                    }
                    Text(String(localized: "restore_action", defaultValue: "Restore Wallet"))
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
            }
            .buttonStyle(.glassProminent)
            .disabled(isRestoring)
        } else {
            Button {
                Task { await performRestore() }
            } label: {
                HStack(spacing: 8) {
                    if isRestoring {
                        ProgressView()
                    } else {
                        Image(systemName: "arrow.down.circle")
                    }
                    Text(String(localized: "restore_action", defaultValue: "Restore Wallet"))
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .background(.blue)
                .foregroundStyle(.white)
                .clipShape(.rect(cornerRadius: 12))
            }
            .disabled(isRestoring)
        }
    }

    private func performRestore() async {
        isRestoring = true
        errorMessage = nil

        do {
            let backup = try await backupService.restoreFromCloud()
            onRestore(backup.mnemonic)
            showSuccess = true
            try await Task.sleep(for: .seconds(1.5))
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
            isRestoring = false
        }
    }
}
