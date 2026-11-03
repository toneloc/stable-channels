import SwiftUI

struct BackupSettingsICloudSection: View {
    let backupService: any BackupServiceProtocol
    @Binding var showingBackupPrompt: Bool
    @Binding var showingExportSheet: Bool
    @Binding var showingImportSheet: Bool
    @Binding var showingDeleteConfirmation: Bool
    @Binding var showingRestoreSheet: Bool
    @Binding var showBackupSuccess: Bool
    @Binding var backupError: String?
    @Binding var isCheckingRemote: Bool
    let onBackupNow: () async -> Void

    var body: some View {
        Section {
            if backupService.hasLocalBackup {
                icloudBackupEnabledSection
            } else {
                icloudBackupDisabledSection
            }
        } header: {
            Text(String(localized: .sectionIcloudBackup))
        }
    }

    @ViewBuilder
    private var icloudBackupEnabledSection: some View {
        HStack {
            Label(
                String(localized: .icloudBackup),
                systemImage: "icloud.fill"
            )
            .foregroundStyle(.blue)
            Spacer()
            Text(String(localized: .enabled))
                .font(.caption)
                .foregroundStyle(.green)
        }

        if let lastBackup = backupService.lastBackupDate {
            HStack {
                Text(String(localized: .lastBackup))
                Spacer()
                Text(lastBackup, style: .relative)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }

        Button {
            Task { await onBackupNow() }
        } label: {
            HStack {
                if isCheckingRemote {
                    ProgressView()
                } else {
                    Image(systemName: showBackupSuccess ? "checkmark.circle.fill" : "arrow.clockwise")
                        .foregroundStyle(showBackupSuccess ? .green : .blue)
                }
                Text(showBackupSuccess
                    ? "Backup complete"
                    : String(localized: .backupNow))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)

        if let error = backupError {
            Text(error)
                .font(.caption)
                .foregroundStyle(.red)
        }

        Button {
            showingExportSheet = true
        } label: {
            HStack {
                Image(systemName: "square.and.arrow.up.on.square")
                    .foregroundStyle(.green)
                Text(String(localized: .exportToFiles))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)

        Button(role: .destructive) {
            showingDeleteConfirmation = true
        } label: {
            HStack {
                Image(systemName: "trash.fill")
                    .foregroundStyle(.red)
                Text(String(localized: .deleteBackup))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }

        Button {
            showingRestoreSheet = true
        } label: {
            HStack {
                Image(systemName: "icloud.and.arrow.down.fill")
                    .foregroundStyle(.blue)
                Text(String(localized: .restoreBackup))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)
    }

    @ViewBuilder
    private var icloudBackupDisabledSection: some View {
        Button {
            showingBackupPrompt = true
        } label: {
            HStack {
                Image(systemName: "icloud.and.arrow.up.fill")
                    .foregroundStyle(.blue)
                Text(String(localized: .buttonEnableIcloudBackup))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)

        Button {
            showingRestoreSheet = true
        } label: {
            HStack {
                Image(systemName: "icloud.and.arrow.down.fill")
                    .foregroundStyle(.blue)
                Text(String(localized: .buttonRestoreIcloud))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)

        Button {
            showingImportSheet = true
        } label: {
            HStack {
                Image(systemName: "square.and.arrow.down.on.square")
                    .foregroundStyle(.green)
                Text(String(localized: .buttonImportBackup))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)
    }
}
