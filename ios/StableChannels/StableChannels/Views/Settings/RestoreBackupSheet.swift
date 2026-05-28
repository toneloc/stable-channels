import SwiftUI

struct RestoreBackupSheet: View {
    enum Mode {
        case iCloud
        case fileImport
    }

    let mode: Mode
    let importedFileURL: URL?
    let onRestore: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var isRestoring = false
    @State private var showSuccess = false
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 32) {
                Spacer()

                Image(systemName: mode == .iCloud ? "icloud.and.arrow.down" : "doc.fill")
                    .font(.system(size: 80))
                    .foregroundStyle(.blue)
                    .shadow(color: .blue.opacity(0.3), radius: 20, x: 0, y: 8)

                Text(mode == .iCloud
                    ? String(localized: "restore_from_icloud", defaultValue: "Restore from iCloud")
                    : String(localized: "restore_from_file", defaultValue: "Restore from File"))
                    .font(.title.bold())

                Text(String(
                    localized: "restore_description",
                    defaultValue: "Restore your wallet from backup."
                ))
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 24)

                // Feature list
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
                Task { await performRestoreAction() }
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
                Task { await performRestoreAction() }
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

    private func performRestoreAction() async {
        isRestoring = true
        errorMessage = nil

        do {
            let backupFile: BackupFile

            switch mode {
            case .iCloud:
                backupFile = try await CloudBackupService.shared.restoreFromCloud()
            case .fileImport:
                guard let url = importedFileURL else {
                    errorMessage = "No file selected"
                    isRestoring = false
                    return
                }
                backupFile = try await restoreFromFile(url: url)
            }

            onRestore(backupFile.mnemonic)
            showSuccess = true
            try await Task.sleep(nanoseconds: 1_500_000_000)
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
            isRestoring = false
        }
    }

    private func restoreFromFile(url _: URL) async throws -> BackupFile {
        throw BackupError.importFailed("Use Export/Import to restore from file")
    }
}
