import SwiftUI

struct RestoreSeedSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppState.self) private var appState

    @Binding var restoreMnemonic: String
    @Binding var wordFields: [String]
    @Binding var isWordFieldsReadOnly: Bool
    @Binding var isImportingSeed: Bool
    @Binding var isRestoring: Bool
    @Binding var restoreError: String?
    let wordCount: Int
    let restoreValid: Bool
    let onCancel: () -> Void

    var body: some View {
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

                    SeedWordGridView(
                        wordFields: wordFields,
                        isReadOnly: isWordFieldsReadOnly,
                        isDisabled: isRestoring,
                        wordCount: wordCount,
                        onWordChanged: { index, word in
                            wordFields[index] = word
                            syncMnemonicFromFields()
                        }
                    )

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
                        onCancel()
                        dismiss()
                    }
                }
            }
        }
    }

    private func syncWordFields(from text: String) {
        wordFields = MnemonicUtils.wordsToFields(MnemonicUtils.parseMnemonic(text))
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

        guard MnemonicUtils.isValidWordCount(input) else {
            isRestoring = false
            restoreError = String(localized: "error_seed_word_count")
            return
        }

        guard MnemonicUtils.hasValidCharacterFormat(input) else {
            isRestoring = false
            restoreError = String(localized: "error_invalid_seed")
            return
        }

        do {
            try await appState.nodeService.start(
                network: .bitcoin,
                esploraURL: appState.chainURL,
                mnemonic: input
            )
            await MainActor.run {
                restoreMnemonic = ""
                wordFields = Array(repeating: "", count: SeedConstants.maxWordCount)
                isWordFieldsReadOnly = false
                appState.refreshBalances()
                isRestoring = false
                dismiss()
            }
        } catch {
            await MainActor.run {
                restoreError = String(localized: "error_restore_failed") + error.localizedDescription
                isRestoring = false
            }
        }
    }
}
