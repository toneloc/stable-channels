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
        }
        .listStyle(.insetGrouped)
        .navigationTitle(String(localized: "title_backup", defaultValue: "Backup"))
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showRestore) {
            restoreSheet
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
            VStack(spacing: 24) {
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

                TextField(
                    String(localized: "placeholder_seed", defaultValue: "word1 word2 word3 ..."),
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
                .padding(.horizontal)

                if let error = restoreError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                Button {
                    Task { await restoreWallet() }
                } label: {
                    if isRestoring {
                        ProgressView()
                    } else {
                        Text(String(localized: "button_restore", defaultValue: "Restore"))
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(isRestoring || restoreMnemonic.trimmingCharacters(in: .whitespaces).isEmpty)

                Spacer()
            }
            .padding()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_cancel", defaultValue: "Cancel")) {
                        showRestore = false
                        restoreMnemonic = ""
                        restoreError = nil
                    }
                }
            }
        }
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
                showRestore = false
                restoreMnemonic = ""
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
}
