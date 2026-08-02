import SwiftUI
import UniformTypeIdentifiers

struct ExportSheet: View {
    @Environment(\.dismiss) private var dismiss

    @State private var passphrase = ""
    @State private var confirmPassphrase = ""
    @State private var isProcessing = false
    @State private var errorMessage: String?
    @State private var exportURL: URL?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    headerSection

                    exportContent
                }
                .padding(20)
            }
            .background(Color(.systemGroupedBackground))
            .navigationTitle(String(localized: "export_backup", defaultValue: "Export Backup"))
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
        }
    }

    @State private var showingShareSheet = false

    // MARK: - Header

    private var headerSection: some View {
        VStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(.ultraThinMaterial)
                    .frame(width: 80, height: 80)

                Image(systemName: "square.and.arrow.up.fill")
                    .font(.system(size: 32))
                    .foregroundStyle(.blue)
            }

            Text(String(localized: "export_title", defaultValue: "Export Your Backup"))
                .font(.title2.bold())
                .foregroundStyle(.primary)

            Text(String(
                localized: "export_description",
                defaultValue: "Add a passphrase to protect your exported backup. You'll need this to restore."
            ))
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
        }
        .padding(.top, 8)
    }

    // MARK: - Export Content

    private var exportContent: some View {
        VStack(spacing: 16) {
            PassphraseCard(
                label: String(localized: "passphrase", defaultValue: "Passphrase"),
                prompt: String(localized: "enter_passphrase", defaultValue: "Enter passphrase"),
                text: $passphrase,
                isNew: true
            )

            PassphraseCard(
                label: String(localized: "confirm_passphrase", defaultValue: "Confirm Passphrase"),
                prompt: String(localized: "confirm_passphrase_hint", defaultValue: "Confirm your passphrase"),
                text: $confirmPassphrase,
                isNew: true
            )

            if !passphrase.isEmpty && passphrase.count < 12 {
                RequirementBadge(
                    text: String(localized: "passphrase_min_length", defaultValue: "At least 12 characters required"),
                    isMet: false
                )
            }

            if !passphrase.isEmpty && !confirmPassphrase.isEmpty && passphrase != confirmPassphrase {
                RequirementBadge(
                    text: String(localized: "passphrase_mismatch", defaultValue: "Passphrases don't match"),
                    isMet: false
                )
            }

            SheetErrorBanner(message: $errorMessage)

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

    // MARK: - Validation

    private var isExportValid: Bool {
        !passphrase.isEmpty && passphrase.count >= 12 && passphrase == confirmPassphrase
    }

    // MARK: - Actions

    private func exportBackup() async {
        isProcessing = true
        defer { isProcessing = false }
        errorMessage = nil

        do {
            guard let mnemonic = NodeService.shared.savedMnemonic else {
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
}
