import SwiftUI

struct BackupSettingsView: View {
    @Environment(AppState.self) private var appState
    let backupService: any BackupServiceProtocol

    // MARK: - UI State

    @State private var showSeedWords = false
    @State private var showRestore = false
    @State private var restoreMnemonic = ""
    @State private var restoreSourceIsCloud = false
    @State private var isRestoring = false
    @State private var restoreError: String?
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
    @State private var showingOverwriteAlert = false
    @State private var isCheckingRemote = false

    // MARK: - Computed Properties

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

    private func importMnemonic(_ mnemonic: String, fromCloud: Bool = false) {
        isImportingSeed = true
        restoreSourceIsCloud = fromCloud
        restoreMnemonic = ""
        wordFields = MnemonicUtils.wordsToFields(MnemonicUtils.parseMnemonic(mnemonic))
        isWordFieldsReadOnly = true
        restoreMnemonic = mnemonic
        showRestore = true
    }

    private func backupNow() async {
        guard appState.nodeService.savedMnemonic != nil else { return }
        isCheckingRemote = true
        let exists = await backupService.checkRemoteBackupExists()
        isCheckingRemote = false
        if exists {
            showingOverwriteAlert = true
        } else {
            await executeBackup()
        }
    }

    private func executeBackup() async {
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
            supportBannerSection
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
                onCancel: { cancelRestore() },
                onSuccess: {
                    if restoreSourceIsCloud {
                        backupService.markLocalBackupAsEnabled()
                    }
                }
            )
        }
        .sheet(isPresented: $showingBackupPrompt) {
            BackupPromptView(backupService: backupService)
        }
        .onChange(of: showingBackupPrompt) { _, newValue in
            if !newValue {
                backupService.refreshStatus()
            }
        }
        .sheet(isPresented: $showingRestoreSheet) {
            RestoreBackupSheet(backupService: backupService) { mnemonic in
                importMnemonic(mnemonic, fromCloud: true)
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
        .nativeTimerAlert(isPresented: $showingOverwriteAlert, title: "Overwrite Existing Backup?") {
            await executeBackup()
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
                SeedDisplayView(words: words)
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

    private var supportBannerSection: some View {
        Section {
            BackupSettingsSupportBanner()
        }
    }

    private var icloudSection: some View {
        BackupSettingsICloudSection(
            backupService: backupService,
            showingBackupPrompt: $showingBackupPrompt,
            showingExportSheet: $showingExportSheet,
            showingImportSheet: $showingImportSheet,
            showingDeleteConfirmation: $showingDeleteConfirmation,
            showingRestoreSheet: $showingRestoreSheet,
            showBackupSuccess: $showBackupSuccess,
            backupError: $backupError,
            isCheckingRemote: $isCheckingRemote,
            onBackupNow: { await backupNow() }
        )
    }
}
