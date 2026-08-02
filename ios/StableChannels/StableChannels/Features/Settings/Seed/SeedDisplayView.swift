import SwiftUI

struct SeedDisplayView: View {
    let words: String

    @State private var copiedSeed = false
    @State private var showCopyWarning = false
    @State private var clipboardClearTask: Task<Void, Never>?
    @State private var clipboardFadeTask: Task<Void, Never>?

    private func copySeedToClipboard() {
        clipboardClearTask?.cancel()
        clipboardFadeTask?.cancel()
        UIPasteboard.general.string = words
        withAnimation { copiedSeed = true }

        clipboardClearTask = Task {
            try? await Task.sleep(for: .seconds(SeedConstants.clipboardClearSeconds))
            if UIPasteboard.general.string == words {
                UIPasteboard.general.string = ""
            }
        }
        clipboardFadeTask = Task {
            try? await Task.sleep(for: .seconds(2))
            withAnimation { self.copiedSeed = false }
        }
    }

    private func cancelClipboardTasks() {
        clipboardClearTask?.cancel()
        clipboardFadeTask?.cancel()
    }

    var body: some View {
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
        .onDisappear {
            cancelClipboardTasks()
        }
    }
}
