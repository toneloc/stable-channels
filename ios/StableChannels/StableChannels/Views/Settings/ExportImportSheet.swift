import SwiftUI
import UniformTypeIdentifiers

extension UTType {
    static var stableBackup: UTType {
        UTType(filenameExtension: "stablebackup") ?? .data
    }
}

struct ExportImportSheet: View {
    enum Mode: String {
        case export
        case importFile
    }

    let mode: Mode
    var onRestore: ((String) -> Void)?

    init(mode: Mode, onRestore: ((String) -> Void)? = nil) {
        self.mode = mode
        self.onRestore = onRestore
    }

    @Environment(\.dismiss) private var dismiss
    @State private var passphrase = ""
    @State private var confirmPassphrase = ""
    @State private var isProcessing = false
    @State private var errorMessage: String?
    @State private var exportURL: URL?
    @State private var showingShareSheet = false
    @State private var showingFilePicker = false
    @State private var selectedFileURL: URL?
    @State private var selectedFileName: String?
    @State private var animateFileSelection = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    headerSection

                    if mode == .export {
                        exportContent
                    } else {
                        importContent
                    }
                }
                .padding(20)
            }
            .background(Color(.systemGroupedBackground))
            .navigationTitle(mode == .export ? String(localized: "export_backup", defaultValue: "Export Backup") :
                String(
                    localized: "import_backup",
                    defaultValue: "Import Backup"
                ))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_cancel", defaultValue: "Cancel")) { dismiss() }
                        .foregroundStyle(.secondary)
                }
            }
            .sheet(isPresented: $showingShareSheet) {
                if let url = exportURL {
                    ShareSheet(items: [url])
                }
            }
            .fileImporter(
                isPresented: $showingFilePicker,
                allowedContentTypes: [.stableBackup],
                allowsMultipleSelection: false
            ) { result in
                handleFileSelection(result)
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

                Image(systemName: mode == .export ? "square.and.arrow.up.fill" : "square.and.arrow.down.fill")
                    .font(.system(size: 32))
                    .foregroundStyle(.blue)
            }

            Text(mode == .export
                ? String(localized: "export_title", defaultValue: "Export Your Backup")
                : String(localized: "import_title", defaultValue: "Restore from Backup"))
                .font(.title2.bold())
                .foregroundStyle(.primary)

            Text(mode == .export
                ? String(
                    localized: "export_description",
                    defaultValue: "Add a passphrase to protect your exported backup. You'll need this to restore."
                )
                : String(
                    localized: "import_description",
                    defaultValue: "Select your .stablebackup file and enter the passphrase used when creating it."
                ))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.top, 8)
    }

    // MARK: - Export

    private var exportContent: some View {
        VStack(spacing: 16) {
            passphraseCard(
                label: String(localized: "passphrase", defaultValue: "Passphrase"),
                placeholder: String(localized: "enter_passphrase", defaultValue: "Enter passphrase"),
                text: $passphrase,
                isNew: true
            )

            passphraseCard(
                label: String(localized: "confirm_passphrase", defaultValue: "Confirm Passphrase"),
                placeholder: String(localized: "confirm_passphrase_hint", defaultValue: "Confirm your passphrase"),
                text: $confirmPassphrase,
                isNew: true
            )

            if !passphrase.isEmpty && passphrase.count < 12 {
                requirementBadge(
                    text: String(localized: "passphrase_min_length", defaultValue: "At least 12 characters required"),
                    isMet: false
                )
            }

            if !passphrase.isEmpty && !confirmPassphrase.isEmpty && passphrase != confirmPassphrase {
                requirementBadge(
                    text: String(localized: "passphrase_mismatch", defaultValue: "Passphrases don't match"),
                    isMet: false
                )
            }

            errorBanner

            Spacer(minLength: 20)

            Button {
                Task { await exportBackup() }
            } label: {
                HStack(spacing: 8) {
                    if isProcessing {
                        ProgressView()
                    } else {
                        Image(systemName: "square.and.arrow.up")
                    }
                    Text(String(localized: "export", defaultValue: "Export"))
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .background(.ultraThinMaterial)
                .foregroundStyle(isExportValid ? .blue : .secondary)
                .clipShape(.rect(cornerRadius: 14))
                .overlay(
                    RoundedRectangle(cornerRadius: 14)
                        .strokeBorder(isExportValid ? Color.blue.opacity(0.3) : Color.clear, lineWidth: 1)
                )
            }
            .disabled(!isExportValid || isProcessing)
        }
    }

    // MARK: - Import

    private var importContent: some View {
        VStack(spacing: 16) {
            filePickerCard

            if selectedFileURL != nil {
                passphraseCard(
                    label: String(localized: "passphrase", defaultValue: "Passphrase"),
                    placeholder: String(localized: "enter_backup_passphrase", defaultValue: "Enter backup passphrase"),
                    text: $passphrase,
                    isNew: false
                )
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            errorBanner

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

    // MARK: - File Picker Card

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

    // MARK: - Passphrase Card

    private func passphraseCard(label: String, placeholder: String, text: Binding<String>, isNew: Bool) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)

            SecureField(placeholder, text: text)
                .textContentType(isNew ? .newPassword : .password)
                .padding(16)
                .background(Color(uiColor: .secondarySystemGroupedBackground))
                .clipShape(.rect(cornerRadius: 12))
        }
    }

    // MARK: - Requirement Badge

    private func requirementBadge(text: String, isMet: Bool) -> some View {
        HStack(spacing: 8) {
            Image(systemName: isMet ? "checkmark.circle.fill" : "circle")
                .font(.caption)
                .foregroundStyle(isMet ? .green : .secondary)

            Text(text)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground).opacity(0.5))
        .clipShape(.rect(cornerRadius: 8))
    }

    // MARK: - Error Banner

    @ViewBuilder
    private var errorBanner: some View {
        if let error = errorMessage {
            HStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)

                Text(error)
                    .font(.subheadline)
                    .foregroundStyle(.red)

                Spacer()
            }
            .padding(14)
            .frame(maxWidth: .infinity)
            .background(Color.red.opacity(0.08))
            .clipShape(.rect(cornerRadius: 12))
        }
    }

    // MARK: - Validation

    private var isExportValid: Bool {
        !passphrase.isEmpty && passphrase.count >= 12 && passphrase == confirmPassphrase
    }

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

    private func exportBackup() async {
        isProcessing = true
        defer { isProcessing = false }
        errorMessage = nil

        do {
            guard let mnemonic = NodeService().savedMnemonic else {
                errorMessage = String(localized: "error_no_seed", defaultValue: "No seed available")
                return
            }
            let encryptedData = try CryptoService.encrypt(mnemonic: mnemonic, passphrase: passphrase).data

            let filename = "stablechannels-backup-\(Date().ISO8601Format()).stablebackup"
            let tempURL = FileManager.default.temporaryDirectory.appendingPathComponent(filename)
            try encryptedData.write(to: tempURL)
            exportURL = tempURL
            showingShareSheet = true
        } catch {
            errorMessage = String(localized: "error_export_failed", defaultValue: "Export failed")
        }
    }

    private func importBackup() async {
        guard let fileURL = selectedFileURL else {
            errorMessage = String(localized: "error_no_file", defaultValue: "No file selected")
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
            onRestore?(backup.mnemonic)
            dismiss()
        } catch {
            errorMessage = String(
                localized: "error_import_failed",
                defaultValue: "Decryption failed. Check passphrase."
            )
        }
    }
}

struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context _: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_: UIActivityViewController, context _: Context) {}
}
