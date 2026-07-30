import SwiftUI
import UniformTypeIdentifiers

struct ImportSheet: View {
    @Environment(\.dismiss) private var dismiss
    var onRestore: ((String) -> Void)?

    @State private var passphrase = ""
    @State private var isProcessing = false
    @State private var errorMessage: String?
    @State private var selectedFileURL: URL?
    @State private var selectedFileName: String?
    @State private var animateFileSelection = false
    @State private var isRateLimited = false
    @State private var lockoutSeconds = 0

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    headerSection

                    filePickerCard

                    if selectedFileURL != nil {
                        PassphraseCard(
                            label: String(localized: "passphrase", defaultValue: "Passphrase"),
                            prompt: String(
                                localized: "enter_backup_passphrase",
                                defaultValue: "Enter backup passphrase"
                            ),
                            text: $passphrase,
                            isNew: false
                        )
                        .transition(.opacity.combined(with: .move(edge: .top)))
                    }

                    SheetErrorBanner(message: $errorMessage)

                    Spacer(minLength: 20)

                    Button {
                        Task { await importBackup() }
                    } label: {
                        HStack(spacing: 8) {
                            if isProcessing {
                                ProgressView()
                            } else {
                                Image(systemName: "arrow.down.circle.fill")
                            }
                            Text(String(localized: "import", defaultValue: "Import"))
                                .fontWeight(.semibold)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 16)
                        .background(.ultraThinMaterial)
                        .foregroundStyle(isImportValid ? .blue : .secondary)
                        .clipShape(.rect(cornerRadius: 14))
                        .overlay(
                            RoundedRectangle(cornerRadius: 14)
                                .strokeBorder(isImportValid ? Color.blue.opacity(0.3) : Color.clear, lineWidth: 1)
                        )
                    }
                    .disabled(!isImportValid || isProcessing)
                }
                .animation(.easeInOut(duration: 0.3), value: selectedFileURL != nil)
            }
            .padding(20)
        }
        .background(Color(.systemGroupedBackground))
        .navigationTitle(String(localized: "import_backup", defaultValue: "Import Backup"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button(String(localized: "button_cancel", defaultValue: "Cancel")) { dismiss() }
                    .foregroundStyle(.secondary)
            }
        }
    }

    // MARK: - Header

    private var headerSection: some View {
        VStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(.ultraThinMaterial)
                    .frame(width: 80, height: 80)

                Image(systemName: "square.and.arrow.down.fill")
                    .font(.system(size: 32))
                    .foregroundStyle(.blue)
            }

            Text(String(localized: "import_title", defaultValue: "Restore from Backup"))
                .font(.title2.bold())
                .foregroundStyle(.primary)

            Text(String(
                localized: "import_description",
                defaultValue: "Select your .stablebackup file and enter the passphrase used when creating it."
            ))
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
        }
        .padding(.top, 8)
    }

    // MARK: - File Picker

    private var filePickerCard: some View {
        Button {
            showingFilePicker = true
        } label: {
            HStack(spacing: 14) {
                ZStack {
                    RoundedRectangle(cornerRadius: 12)
                        .fill(selectedFileURL != nil ? Color.green.opacity(0.1) : Color.blue.opacity(0.1))
                        .frame(width: 52, height: 52)

                    Image(systemName: selectedFileURL != nil ? "checkmark.circle.fill" : "doc.fill")
                        .font(.title2)
                        .foregroundStyle(selectedFileURL != nil ? .green : .blue)
                        .scaleEffect(animateFileSelection ? 1.1 : 1.0)
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text(selectedFileURL != nil
                        ? (selectedFileName ?? "File selected")
                        : String(localized: "select_backup_file", defaultValue: "Select .stablebackup File"))
                        .font(.body)
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    Text(selectedFileURL != nil
                        ? String(localized: "file_ready", defaultValue: "File ready to restore")
                        : String(localized: "tap_to_select", defaultValue: "Tap to browse your files"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(16)
            .background(.ultraThinMaterial)
            .clipShape(.rect(cornerRadius: 16))
            .overlay(
                RoundedRectangle(cornerRadius: 16)
                    .strokeBorder(selectedFileURL != nil ? Color.green.opacity(0.3) : Color.clear, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }

    @State private var showingFilePicker = false

    // MARK: - Validation

    private var isImportValid: Bool {
        selectedFileURL != nil && !passphrase.isEmpty
    }

    // MARK: - Actions

    private func handleFileSelection(_ result: Result<[URL], Error>) {
        switch result {
        case .success(let urls):
            if let url = urls.first {
                selectedFileURL = url
                selectedFileName = url.lastPathComponent
                withAnimation(.easeInOut(duration: 0.2)) {
                    animateFileSelection = true
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        animateFileSelection = false
                    }
                }
            }
        case .failure(let error):
            errorMessage = error.localizedDescription
        }
    }

    private func importBackup() async {
        guard let fileURL = selectedFileURL else {
            errorMessage = String(localized: "error_no_file", defaultValue: "No file selected")
            return
        }

        // Check rate limit before attempting decryption
        if RateLimitService.shared.isLocked {
            lockoutSeconds = RateLimitService.shared.lockoutRemainingSeconds
            isRateLimited = true
            errorMessage = "\(String(localized: "error_rate_limited", defaultValue: "Too many attempts. Try again in")) \(lockoutSeconds)s"
            return
        }

        isProcessing = true
        defer { isProcessing = false }
        errorMessage = nil

        do {
            let accessing = fileURL.startAccessingSecurityScopedResource()
            defer {
                if accessing {
                    fileURL.stopAccessingSecurityScopedResource()
                }
            }

            let encryptedData = try Data(contentsOf: fileURL)
            let backup = try CryptoService.decrypt(data: encryptedData, passphrase: passphrase)

            // Success — reset rate limit
            RateLimitService.shared.recordSuccessfulAttempt()
            onRestore?(backup.mnemonic)
            dismiss()
        } catch let error as CryptoError {
            RateLimitService.shared.recordFailedAttempt()

            if case .invalidMagicBytes = error {
                errorMessage = String(localized: "error_invalid_backup", defaultValue: "Invalid backup file")
            } else if case .unsupportedVersion = error {
                errorMessage = String(localized: "error_old_backup", defaultValue: "Backup version not supported")
            } else {
                let remaining = RateLimitService.shared.attemptsRemaining
                if RateLimitService.shared.isLocked {
                    lockoutSeconds = RateLimitService.shared.lockoutRemainingSeconds
                    errorMessage = "\(String(localized: "error_wrong_passphrase", defaultValue: "Wrong passphrase. \(remaining) attempts remaining")) "
                        + "\(String(localized: "error_locked_for", defaultValue: "Locked for")) \(lockoutSeconds)s"
                } else {
                    errorMessage = "\(String(localized: "error_import_failed", defaultValue: "Decryption failed. Check passphrase.")) "
                        + "(\(remaining) \(String(localized: "label_attempts_left", defaultValue: "attempts left")))"
                }
            }
        } catch {
            RateLimitService.shared.recordFailedAttempt()
            errorMessage = String(
                localized: "error_import_failed",
                defaultValue: "Decryption failed. Check passphrase."
            )
        }
    }
}
