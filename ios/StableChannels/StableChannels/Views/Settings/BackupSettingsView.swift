import SwiftUI

struct BackupSettingsView: View {
    @Environment(AppState.self) private var appState
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

    var body: some View {
        List {
            // View Seed
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
            } header: {
                Text(String(localized: "section_backup_seed", defaultValue: "Backup"))
            } footer: {
                Text(String(
                    localized: "info_backup_seed",
                    defaultValue: "Write these words down and store them safely. Anyone with these words can access your funds."
                ))
            }

            // Seed Words Display
            if showSeedWords, let words = appState.nodeService.savedMnemonic, !words.isEmpty {
                Section {
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

                        Button {
                            showCopyWarning = true
                        } label: {
                            HStack {
                                Image(systemName: copiedSeed ? "checkmark" : "doc.on.doc")
                                Text(copiedSeed
                                    ? String(localized: "button_copied", defaultValue: "Copied")
                                    : String(localized: "button_copy_seed", defaultValue: "Copy to Clipboard"))
                            }
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                            .background(Color(uiColor: .secondarySystemGroupedBackground))
                            .clipShape(RoundedRectangle(cornerRadius: 10))
                        }
                        .disabled(copiedSeed)
                    }
                    .padding(.vertical, 4)
                } header: {
                    Text(String(localized: "section_seed_phrase", defaultValue: "Seed Phrase"))
                }
            }

            // Restore
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

            // iCloud Backup Section
            Section {
                if CloudBackupService.shared.backupExists {
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

                    if let lastBackup = CloudBackupService.shared.lastBackupDate {
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
                } else {
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
                            Image(systemName: "square.and.arrow.up.on.square")
                                .foregroundStyle(.green)
                            Text(String(localized: "button_export_import", defaultValue: "Export / Import Backup"))
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .foregroundStyle(.primary)
                }
            } header: {
                Text(String(localized: "section_icloud_backup", defaultValue: "iCloud"))
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle(String(localized: "title_backup", defaultValue: "Backup"))
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showRestore) {
            restoreSheet
        }
        .sheet(isPresented: $showingBackupPrompt) {
            BackupPromptView()
        }
        .onChange(of: showingBackupPrompt) { _, newValue in
            if !newValue {
                CloudBackupService.shared.refreshStatus()
            }
        }
        .sheet(isPresented: $showingRestoreSheet) {
            RestoreBackupSheet(mode: .iCloud, importedFileURL: nil) { mnemonic in
                isImportingSeed = true
                restoreMnemonic = ""
                var newFields = Array(repeating: "", count: 24)
                let words = mnemonic.split(separator: " ").map(String.init)
                for (index, word) in words.enumerated() where index < 24 {
                    newFields[index] = word
                }
                wordFields = newFields
                isWordFieldsReadOnly = true
                restoreMnemonic = mnemonic
                showRestore = true
            }
        }
        .sheet(isPresented: $showingExportSheet) {
            ExportImportSheet(mode: .export)
        }
        .sheet(isPresented: $showingImportSheet) {
            ExportImportSheet(mode: .importFile) { mnemonic in
                isImportingSeed = true
                restoreMnemonic = ""
                var newFields = Array(repeating: "", count: 24)
                let words = mnemonic.split(separator: " ").map(String.init)
                for (index, word) in words.enumerated() where index < 24 {
                    newFields[index] = word
                }
                wordFields = newFields
                isWordFieldsReadOnly = true
                restoreMnemonic = mnemonic
                showRestore = true
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
        .confirmationDialog(
            String(localized: "warning_copy_seed_title", defaultValue: "Copy Seed Words?"),
            isPresented: $showCopyWarning,
            titleVisibility: .visible
        ) {
            Button(String(localized: "button_copy_anyway", defaultValue: "Copy Anyway")) {
                copySeedToClipboard()
            }
            Button(String(localized: "button_cancel", defaultValue: "Cancel"), role: .cancel) { }
        } message: {
            Text(String(
                localized: "warning_copy_seed_message",
                defaultValue: "Clipboard is shared with other apps. Consider writing down your seed instead."
            ))
        }
    }

    private func copySeedToClipboard() {
        guard let words = appState.nodeService.savedMnemonic else { return }
        UIPasteboard.general.string = words
        withAnimation { copiedSeed = true }

        // Clear clipboard after 60 seconds for security
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
            if UIPasteboard.general.string == words {
                UIPasteboard.general.string = ""
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            withAnimation { copiedSeed = false }
        }
    }

    private var restoreSheet: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    Image(systemName: "arrow.uturn.backward.circle.fill")
                        .font(.system(size: 48))
                        .foregroundStyle(.orange)
                    Text(String(localized: "title_restore_seed", defaultValue: "Restore from Seed"))
                        .font(.title2.bold())
                    Text(String(
                        localized: "instruction_restore",
                        defaultValue: "Enter your 12 or 24-word seed phrase."
                    ))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                    // Single input for paste detection
                    TextField(
                        String(localized: "placeholder_seed", defaultValue: "Paste your seed phrase here"),
                        text: $restoreMnemonic,
                        axis: .vertical
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(3...5)
                    .font(.system(.body, design: .monospaced))
                    .padding()
                    .background(Color(uiColor: .secondarySystemGroupedBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                    .onChange(of: restoreMnemonic) { _, newValue in
                        if isImportingSeed {
                            isImportingSeed = false
                            return
                        }
                        syncWordFields(from: newValue)
                    }
                    .padding(.horizontal)
                    .disabled(isRestoring)

                    // Word grid
                    wordGrid
                        .id(wordFields.map(\.hashValue).hashValue)

                    if let error = restoreError {
                        Text(error)
                            .font(.caption)
                            .foregroundStyle(.red)
                    }

                    Button {
                        Task { await restoreWallet() }
                    } label: {
                        if isRestoring {
                            HStack(spacing: 8) {
                                ProgressView()
                                Text(String(localized: "restoring", defaultValue: "Restoring..."))
                            }
                        } else {
                            Text(String(localized: "button_restore", defaultValue: "Restore"))
                        }
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 16)
                    .background(.ultraThinMaterial)
                    .foregroundStyle(restoreValid ? .blue : .secondary)
                    .clipShape(.rect(cornerRadius: 14))
                    .overlay(
                        RoundedRectangle(cornerRadius: 14)
                            .strokeBorder(restoreValid ? Color.blue.opacity(0.3) : Color.clear, lineWidth: 1)
                    )
                    .disabled(!restoreValid || isRestoring)

                    Spacer()
                }
                .padding()
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_cancel", defaultValue: "Cancel")) {
                        restoreMnemonic = ""
                        showRestore = false
                        restoreError = nil
                        wordFields = Array(repeating: "", count: 24)
                        isWordFieldsReadOnly = false
                    }
                }
            }
        }
    }

    @State private var wordFields: [String] = Array(repeating: "", count: 24)
    @State private var isWordFieldsReadOnly = false
    @State private var isImportingSeed = false

    private var wordGrid: some View {
        let wordCount = detectedWordCount
        let columns = wordCount == 12
            ? [GridItem(.flexible()), GridItem(.flexible())]
            : [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())]

        return LazyVGrid(columns: columns, spacing: 8) {
            ForEach(0..<wordCount, id: \.self) { index in
                HStack(spacing: 4) {
                    Text("\(index + 1).")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .frame(width: 18, alignment: .trailing)

                    if isWordFieldsReadOnly || isRestoring {
                        Text(wordFields[index])
                            .font(.system(.caption, design: .monospaced))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(Color(uiColor: .tertiarySystemGroupedBackground))
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                    } else {
                        TextField("", text: $wordFields[index])
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.system(.caption, design: .monospaced))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(Color(uiColor: .tertiarySystemGroupedBackground))
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .onChange(of: wordFields[index]) { _, _ in
                                isWordFieldsReadOnly = false
                                syncMnemonicFromFields()
                            }
                    }
                }
                .padding(.vertical, 2)
            }
        }
        .padding(.horizontal)
        .animation(.easeInOut, value: wordCount)
        .disabled(isRestoring)
    }

    private var detectedWordCount: Int {
        let words = restoreMnemonic.split(separator: " ").map(String.init).filter { !$0.isEmpty }
        let count = words.count
        return (count == 12 || count == 24) ? count : 12
    }

    private var restoreValid: Bool {
        let filledCount = wordFields.filter { !$0.isEmpty }.count
        return filledCount == 12 || filledCount == 24
    }

    private func syncWordFields(from text: String) {
        var newFields = Array(repeating: "", count: 24)
        let words = text.split(separator: " ").map(String.init)
        for (index, word) in words.enumerated() where index < 24 {
            newFields[index] = word
        }
        wordFields = newFields
        isWordFieldsReadOnly = true
    }

    private func syncMnemonicFromFields() {
        isWordFieldsReadOnly = false
        restoreMnemonic = wordFields.filter { !$0.isEmpty }.joined(separator: " ")
    }

    private func restoreWallet() async {
        isRestoring = true
        restoreError = nil

        let input = restoreMnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
        let wordCount = input.split(separator: " ").count
        guard wordCount == 12 || wordCount == 24 else {
            isRestoring = false
            restoreError = String(
                localized: "error_seed_word_count",
                defaultValue: "Seed phrase must be 12 or 24 words"
            )
            return
        }

        // Validate mnemonic format before stopping node
        do {
            // Attempt initialization in isolated flow - if fails, node keeps running
            try await appState.nodeService.start(
                network: .bitcoin,
                esploraURL: appState.chainURL,
                mnemonic: input
            )
            await MainActor.run {
                restoreMnemonic = ""
                showRestore = false
                wordFields = Array(repeating: "", count: 24)
                isWordFieldsReadOnly = false
                appState.refreshBalances()
                isRestoring = false
            }
        } catch {
            // Node start failed - old state preserved, user notified
            await MainActor.run {
                restoreError = String(
                    localized: "error_restore_failed",
                    defaultValue: "Restore failed: "
                ) + error.localizedDescription
                isRestoring = false
            }
        }
    }

    private func backupNow() async {
        guard let mnemonic = appState.nodeService.savedMnemonic else { return }
        backupError = nil
        do {
            try await CloudBackupService.shared.saveBackupToCloud()
            showBackupSuccess = true
            try await Task.sleep(nanoseconds: 1_500_000_000)
            showBackupSuccess = false
        } catch {
            backupError = error.localizedDescription
        }
    }

    private func deleteBackup() async {
        do {
            try await CloudBackupService.shared.deleteBackup()
        } catch {
            // Handle error silently or log
        }
    }
}
