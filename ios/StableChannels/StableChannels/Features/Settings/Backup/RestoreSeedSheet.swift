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
    let onCancel: () -> Void
    let onSuccess: () -> Void

    @State private var showForceCloseConfirm = false
    @State private var showGuardUnavailableConfirm = false

    private var wordCount: Int {
        MnemonicUtils.detectWordCount(restoreMnemonic)
    }

    private var restoreValid: Bool {
        let filledCount = wordFields.filter { !$0.isEmpty }.count
        return filledCount == SeedConstants.wordCount12 || filledCount == SeedConstants.wordCount24
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    headerSection

                    seedTextField
                        .onTapGesture {
                            UIApplication.shared.sendAction(
                                #selector(UIResponder.resignFirstResponder),
                                to: nil,
                                from: nil,
                                for: nil
                            )
                        }

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
                                Text(String(localized: .restoring))
                            }
                        } else {
                            Text(String(localized: .buttonRestore))
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
                    Button(String(localized: .buttonCancel)) {
                        onCancel()
                        dismiss()
                    }
                }
            }
            .alert(
                String(localized: .titleOpenChannelDetected),
                isPresented: $showForceCloseConfirm
            ) {
                Button(String(localized: .buttonCancel), role: .cancel) {}
                Button(
                    String(localized: .buttonRestoreAnyway),
                    role: .destructive
                ) {
                    Task { await restoreWallet(acknowledgeForceClose: true) }
                }
            } message: {
                Text(String(localized: .messageRestoreForceClose))
            }
            .alert(
                String(localized: .titleChannelCheckUnavailable),
                isPresented: $showGuardUnavailableConfirm
            ) {
                Button(String(localized: .buttonCancel), role: .cancel) {}
                Button(
                    String(localized: .buttonContinueAnyway),
                    role: .destructive
                ) {
                    Task { await restoreWallet(acknowledgeForceClose: true) }
                }
            } message: {
                Text(String(localized: .messageChannelCheckUnavailable))
            }
        }
    }

    // MARK: - Subviews

    private var headerSection: some View {
        VStack(spacing: 16) {
            Image(systemName: "arrow.uturn.backward.circle.fill")
                .font(.system(size: 48))
                .foregroundStyle(.orange)

            Text(String(localized: .titleRestoreSeed))
                .font(.title2.bold())

            warningBanner

            Text(String(localized: .instructionRestore))
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
    }

    private var warningBanner: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text("Partial Recovery Warning")
                    .font(.headline)
                Spacer()
            }

            Text(
                "This recovery will restore onchain funds but NOT Lightning channel state. Lightning funds will be lost and may require LSP force-close."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            Text("Please withdraw all BTC before proceeding. Existing wallet data will be completely overwritten.")
                .font(.caption)
                .foregroundStyle(.red)
                .fontWeight(.semibold)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding()
        .background(.orange.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    private var seedTextField: some View {
        TextField(
            String(localized: .placeholderSeed),
            text: $restoreMnemonic,
            axis: .vertical
        )
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .lineLimit(5...10)
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
        .disabled(isRestoring)
    }

    private func syncWordFields(from text: String) {
        wordFields = MnemonicUtils.wordsToFields(MnemonicUtils.parseMnemonic(text))
        isWordFieldsReadOnly = true
    }

    private func syncMnemonicFromFields() {
        isWordFieldsReadOnly = false
        restoreMnemonic = wordFields.filter { !$0.isEmpty }.joined(separator: " ")
    }

    private func restoreWallet(acknowledgeForceClose: Bool = false) async {
        isRestoring = true
        restoreError = nil

        let input = restoreMnemonic.trimmingCharacters(in: .whitespacesAndNewlines)

        guard MnemonicUtils.isValidWordCount(input) else {
            isRestoring = false
            restoreError = String(localized: .errorSeedWordCount)
            return
        }

        do {
            try await appState.restoreWalletFromMnemonic(
                input,
                acknowledgeForceClose: acknowledgeForceClose
            )
            restoreMnemonic = ""
            wordFields = Array(repeating: "", count: SeedConstants.maxWordCount)
            isWordFieldsReadOnly = false
            isRestoring = false
            onSuccess()
            dismiss()
        } catch AppState.WalletRestoreError.activeChannelDetected {
            // Divergence guard tripped: restoring would force-close a live
            // channel. Ask the user to opt in explicitly.
            isRestoring = false
            showForceCloseConfirm = true
        } catch AppState.WalletRestoreError.channelCheckUnavailable {
            // Guard couldn't run — fail-warn instead of silently proceeding.
            isRestoring = false
            showGuardUnavailableConfirm = true
        } catch {
            restoreError = String(localized: .errorRestoreFailed(error.localizedDescription))
            isRestoring = false
        }
    }
}
