import SwiftUI
import UniformTypeIdentifiers

// MARK: - Shared Types & Helpers

extension UTType {
    static var stableBackup: UTType {
        UTType(filenameExtension: "stablebackup") ?? .data
    }
}

struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context _: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_: UIActivityViewController, context _: Context) {}
}

// MARK: - Shared UI Components

struct PassphraseCard: View {
    let label: String
    let prompt: String
    @Binding var text: String
    let isNew: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)

            SecureField(prompt, text: $text)
                .textContentType(isNew ? .newPassword : .password)
                .padding(16)
                .background(Color(uiColor: .secondarySystemGroupedBackground))
                .clipShape(.rect(cornerRadius: 12))
        }
    }
}

struct RequirementBadge: View {
    let text: String
    let isMet: Bool

    var body: some View {
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
}

struct SheetErrorBanner: View {
    @Binding var message: String?

    var body: some View {
        if let error = message {
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
}
