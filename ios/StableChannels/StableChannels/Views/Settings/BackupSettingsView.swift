import SwiftUI

struct BackupSettingsView: View {
    @Environment(AppState.self) private var appState
    private let backupService = CloudBackupService.shared

    // MARK: - UI State

    @State private var showSeedWords = false
    @State private var showRestore = false
    @State private var restoreMnemonic = ""
    @State private var isRestoring = false
    @State private var restoreError: String?
    @State private var copiedSeed = false
    @State private var showCopyWarning = false
    @State private var showingBackupPrompt = false
    @State private var showingExportSheet = false
    @State private var showingImportSheet = false
    @State private var showingDeleteConfirmation = false
    @State private var showBackupSuccess = false
    @State private var backupError: String?
    @State private var showingRestoreSheet = false
    @State private var wordFields: [String] = Array(repeating: "", count: SeedConstants.maxWordCount)
    @State private var isWordFieldsReadOnly = false
    @State private var isImportingSeed = false

    // MARK: - Computed Properties

    private var detectedWordCount: Int {
        MnemonicUtils.detectWordCount(restoreMnemonic)
    }

    private var restoreValid: Bool {
        let filledCount = wordFields.filter { !$0.isEmpty }.count
        let validCount = (filledCount == SeedConstants.wordCount12 || filledCount == SeedConstants.wordCount24)
        guard validCount else { return false }
        return MnemonicUtils.hasValidCharacterFormat(restoreMnemonic)
    }

    // MARK: - Actions

    private func cancelRestore() {
        restoreMnemonic = ""
        showRestore = false
        restoreError = nil
        wordFields = Array(repeating: "", count: SeedConstants.maxWordCount)
        isWordFieldsReadOnly = false
    }

    private func importMnemonic(_ mnemonic: String) {
        isImportingSeed = true
        restoreMnemonic = ""
        wordFields = MnemonicUtils.wordsToFields(MnemonicUtils.parseMnemonic(mnemonic))
        isWordFieldsReadOnly = true
        restoreMnemonic = mnemonic
        showRestore = true
    }

    private func copySeedToClipboard() {
        guard let words = appState.nodeService.savedMnemonic else { return }
        UIPasteboard.general.string = words
        withAnimation { copiedSeed = true }

        Task {
            try? await Task.sleep(for: .seconds(SeedConstants.clipboardClearSeconds))
            if UIPasteboard.general.string == words {
                UIPasteboard.general.string = ""
            }
        }
        Task {
            try? await Task.sleep(for: .seconds(2))
            withAnimation { self.copiedSeed = false }
        }
    }

    private func backupNow() async {
        guard appState.nodeService.savedMnemonic != nil else { return }
        backupError = nil
        do {
            try await backupService.saveBackupToCloud()
            showBackupSuccess = true
            try await Task.sleep(for: .seconds(1.5))
            showBackupSuccess = false
        } catch {
            backupError = error.localizedDescription
        }
    }

    private func deleteBackup() async {
        do {
            try await backupService.deleteBackup()
        } catch {
            print("Delete backup failed: \(error.localizedDescription)")
        }
    }

    // MARK: - Body

    var body: some View {
        List {
            seedSection
            restoreSection
            icloudSection
        }
        .listStyle(.insetGrouped)
        .navigationTitle(String(localized: "title_backup", defaultValue: "Backup"))
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showRestore) {
            RestoreSeedSheet(
                restoreMnemonic: $restoreMnemonic,
                wordFields: $wordFields,
                isWordFieldsReadOnly: $isWordFieldsReadOnly,
                isImportingSeed: $isImportingSeed,
                isRestoring: $isRestoring,
                restoreError: $restoreError,
                wordCount: detectedWordCount,
                restoreValid: restoreValid,
                onCancel: { cancelRestore() }
            )
        }
        .sheet(isPresented: $showingBackupPrompt) {
            BackupPromptView()
        }
        .onChange(of: showingBackupPrompt) { _, newValue in
            if !newValue {
                backupService.refreshStatus()
            }
        }
        .sheet(isPresented: $showingRestoreSheet) {
            RestoreBackupSheet { mnemonic in
                importMnemonic(mnemonic)
            }
        }
        .sheet(isPresented: $showingExportSheet) {
            ExportImportSheet(mode: .export)
        }
        .sheet(isPresented: $showingImportSheet) {
            ExportImportSheet(mode: .importFile) { mnemonic in
                importMnemonic(mnemonic)
            }
        }
        .alert("Delete Backup?", isPresented: $showingDeleteConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                Task { await deleteBackup() }
            }
        } message: {
            Text("You'll need your seed phrase to restore. This cannot be undone.")
        }
    }

    // MARK: - Sections

    private var seedSection: some View {
        Section {
            Button {
                showSeedWords.toggle()
            } label: {
                HStack {
                    Image(systemName: showSeedWords ? "eye.slash.fill" : "eye.fill")
                        .foregroundStyle(Color.stablePrimary)
                    Text(showSeedWords
                        ? String(localized: "button_hide_seed", defaultValue: "Hide Seed Words")
                        : String(localized: "button_view_seed", defaultValue: "View Seed Words"))
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .foregroundStyle(.primary)

            if showSeedWords, let words = appState.nodeService.savedMnemonic, !words.isEmpty {
                seedWordsDisplay(words: words)
            }
        } header: {
            Text(String(localized: "section_backup_seed", defaultValue: "Backup"))
        } footer: {
            Text(String(
                localized: "info_backup_seed",
                defaultValue: "Write these words down and store them safely. Anyone with these words can access your funds."
            ))
        }
    }

    private func seedWordsDisplay(words: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(String(localized: "warning_seed", defaultValue: "Never share your seed words"))
                    .font(.caption.bold())
                    .foregroundStyle(.orange)
            }

            let wordList = words.split(separator: " ").map(String.init)
            LazyVGrid(columns: [
                GridItem(.flexible()),
                GridItem(.flexible()),
                GridItem(.flexible())
            ], spacing: 6) {
                ForEach(Array(wordList.enumerated()), id: \.offset) { index, word in
                    HStack(spacing: 4) {
                        Text("\(index + 1).")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .frame(width: 18, alignment: .trailing)
                        Text(word)
                            .font(.system(.caption, design: .monospaced))
                    }
                    .padding(.vertical, 6)
                    .padding(.horizontal, 6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color(uiColor: .secondarySystemGroupedBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
            }

            if !copiedSeed && !showCopyWarning {
                Button {
                    showCopyWarning = true
                } label: {
                    HStack {
                        Image(systemName: "doc.on.doc")
                        Text(String(localized: "button_copy_seed", defaultValue: "Copy to Clipboard"))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .background(Color(uiColor: .secondarySystemGroupedBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 10))
                }
            }

            if copiedSeed {
                HStack {
                    Image(systemName: "checkmark")
                    Text(String(localized: "button_copied", defaultValue: "Copied"))
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .background(Color.green.opacity(0.15))
                .clipShape(RoundedRectangle(cornerRadius: 10))
            }

            if showCopyWarning {
                VStack(spacing: 8) {
                    Text(String(localized: "warning_copy_seed_title", defaultValue: "Copy Seed Words?"))
                        .font(.caption.bold())

                    Text(String(
                        localized: "warning_copy_seed_message",
                        defaultValue: "Clipboard is shared with other apps."
                    ))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                    HStack(spacing: 12) {
                        Button(String(localized: "button_cancel", defaultValue: "Cancel")) {
                            showCopyWarning = false
                        }
                        .font(.caption)
                        .buttonStyle(.bordered)
                        .controlSize(.small)

                        Button(String(localized: "button_copy_anyway", defaultValue: "Copy Anyway")) {
                            copySeedToClipboard()
                            showCopyWarning = false
                        }
                        .font(.caption)
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity)
                .background(Color(uiColor: .tertiarySystemGroupedBackground))
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .transition(.opacity.combined(with: .scale(scale: 0.95)))
            }
        }
        .padding(.vertical, 4)
        .animation(.easeInOut(duration: 0.2), value: showCopyWarning)
    }

    private var restoreSection: some View {
        Section {
            Button {
                showRestore = true
            } label: {
                HStack {
                    Image(systemName: "arrow.uturn.backward.circle.fill")
                        .foregroundStyle(.orange)
                    Text(String(localized: "button_restore_seed", defaultValue: "Restore from Seed"))
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .foregroundStyle(.primary)
        } header: {
            Text(String(localized: "section_restore", defaultValue: "Restore"))
        } footer: {
            Text(String(
                localized: "info_restore",
                defaultValue: "Restore will stop your current node and start fresh."
            ))
        }
    }

    private var icloudSection: some View {
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
                Task { await backupNow() }
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
