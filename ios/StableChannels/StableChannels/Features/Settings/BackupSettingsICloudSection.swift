import SwiftUI

struct BackupSettingsICloudSection: View {
    let backupService: CloudBackupService
    @Binding var showingBackupPrompt: Bool
    @Binding var showingExportSheet: Bool
    @Binding var showingImportSheet: Bool
    @Binding var showingDeleteConfirmation: Bool
    @Binding var showingRestoreSheet: Bool
    @Binding var showBackupSuccess: Bool
    @Binding var backupError: String?
    let onBackupNow: () async -> Void

    var body: some View {
        Section {
            if backupService.backupExists {
                icloudBackupEnabledSection
            } else {
                icloudBackupDisabledSection
            }
        } header: {
            Text(String(localized: "section_icloud_backup", defaultValue: "iCloud"))
        }
    }

    private var icloudBackupEnabledSection: some View {
        Group {
            HStack {
                Label(
                    String(localized: "icloud_backup", defaultValue: "iCloud Backup"),
                    systemImage: "icloud.fill"
                )
                .foregroundStyle(.blue)
                Spacer()
                Text(String(localized: "enabled", defaultValue: "Enabled"))
                    .font(.caption)
                    .foregroundStyle(.green)
            }

            if let lastBackup = backupService.lastBackupDate {
                HStack {
                    Text(String(localized: "last_backup", defaultValue: "Last backup"))
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
                    Image(systemName: showBackupSuccess ? "checkmark.circle.fill" : "arrow.clockwise")
                        .foregroundStyle(showBackupSuccess ? .green : .blue)
                    Text(showBackupSuccess
                        ? "Backup complete"
                        : String(localized: "backup_now", defaultValue: "Backup Now"))
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
                    Text(String(localized: "export_to_files", defaultValue: "Export to Files"))
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
                    Text(String(localized: "delete_backup", defaultValue: "Delete Backup"))
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
                    Text(String(localized: "restore_backup", defaultValue: "Restore Backup"))
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .foregroundStyle(.primary)
        }
    }

    private var icloudBackupDisabledSection: some View {
        Group {
            Button {
                showingBackupPrompt = true
            } label: {
                HStack {
                    Image(systemName: "icloud.and.arrow.up.fill")
                        .foregroundStyle(.blue)
                    Text(String(localized: "button_enable_icloud_backup", defaultValue: "Backup to iCloud"))
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
                    Text(String(localized: "button_restore_icloud", defaultValue: "Restore from iCloud"))
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
                    Text(String(localized: "button_import_backup", defaultValue: "Import from File"))
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .foregroundStyle(.primary)
        }
    }
}
