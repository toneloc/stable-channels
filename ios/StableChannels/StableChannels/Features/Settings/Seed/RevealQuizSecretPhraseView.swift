import SwiftUI

// MARK: - Secret Phrase Revealed View (with In-Place Blur & Tap-to-Reveal)

struct RevealQuizSecretPhraseView: View {
    let mnemonic: String
    let onDone: () -> Void

    @State private var isRevealed: Bool = false
    @State private var copiedSeed = false
    @State private var showCopyWarning = false
    @State private var clipboardClearTask: Task<Void, Never>?
    @State private var clipboardFadeTask: Task<Void, Never>?

    private var wordList: [String] {
        mnemonic.split(separator: " ").map(String.init)
    }

    private func copySeedToClipboard() {
        clipboardClearTask?.cancel()
        clipboardFadeTask?.cancel()

        // 1. Apple Native Clipboard Security: Expiration Date & Local-Only (prevents iCloud Universal Clipboard
        // leakage)
        let expirationDate = Date().addingTimeInterval(Double(SeedConstants.clipboardClearSeconds))
        UIPasteboard.general.setItems(
            [["public.utf8-plain-text": mnemonic]],
            options: [
                .expirationDate: expirationDate,
                .localOnly: true
            ]
        )

        // 2. In-App Fallback Timer to wipe pasteboard string
        clipboardClearTask = Task {
            try? await Task.sleep(for: .seconds(SeedConstants.clipboardClearSeconds))
            if UIPasteboard.general.string == mnemonic {
                UIPasteboard.general.string = ""
            }
        }

        let generator = UINotificationFeedbackGenerator()
        generator.notificationOccurred(.success)

        withAnimation(.easeInOut(duration: 0.2)) {
            copiedSeed = true
        }

        clipboardFadeTask = Task {
            try? await Task.sleep(for: .seconds(2.5))
            withAnimation(.easeInOut(duration: 0.2)) {
                self.copiedSeed = false
            }
        }
    }

    private func cancelClipboardTasks() {
        clipboardClearTask?.cancel()
        clipboardFadeTask?.cancel()
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header Subtitle
            Text("Your Secret Recovery Phrase gives full access to your wallet. Do not share it with anyone.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 24)
                .padding(.top, 16)
                .padding(.bottom, 20)
                .lineSpacing(3)

            Spacer()

            // Seed Grid Area with In-Place Blur & Tap to Reveal Overlay
            ZStack {
                // The Word Grid (3 columns, rounded pills)
                LazyVGrid(columns: [
                    GridItem(.flexible(), spacing: 8),
                    GridItem(.flexible(), spacing: 8),
                    GridItem(.flexible(), spacing: 8)
                ], spacing: 8) {
                    ForEach(Array(wordList.enumerated()), id: \.offset) { index, word in
                        HStack(spacing: 4) {
                            Text("\(index + 1).")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .frame(width: 20, alignment: .trailing)

                            Text(isRevealed ? word : "•••••")
                                .font(.system(.subheadline, design: .monospaced).weight(.medium))
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                        }
                        .padding(.vertical, 10)
                        .padding(.horizontal, 8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color(uiColor: .secondarySystemGroupedBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .stroke(Color.white.opacity(0.06), lineWidth: 1)
                        )
                    }
                }
                .blur(radius: isRevealed ? 0 : 16)
                .allowsHitTesting(isRevealed)

                // Tap to Reveal Privacy Shield Overlay
                if !isRevealed {
                    Button {
                        let generator = UIImpactFeedbackGenerator(style: .medium)
                        generator.impactOccurred()
                        withAnimation(.spring(response: 0.35, dampingFraction: 0.82)) {
                            isRevealed = true
                        }
                    } label: {
                        VStack(spacing: 12) {
                            Image(systemName: "eye.slash")
                                .font(.system(size: 32, weight: .medium))
                                .foregroundStyle(.white)

                            Text("Tap to reveal")
                                .font(.headline.weight(.bold))
                                .foregroundStyle(.white)

                            Text("Make sure no one is watching your screen.")
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.center)
                                .padding(.horizontal, 24)
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(Color.black.opacity(0.4))
                        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .transition(.opacity)
                }
            }
            .frame(minHeight: 220)
            .padding(.horizontal, 20)

            // Below Grid Actions (Symmetric Copy on Left, Hide on Right)
            if isRevealed {
                HStack(spacing: 12) {
                    // Left: Copy Button
                    Button {
                        showCopyWarning = true
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: copiedSeed ? "checkmark" : "doc.on.doc")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(copiedSeed ? Color.green : Color.white)
                                .animation(.spring(response: 0.3, dampingFraction: 0.7), value: copiedSeed)

                            Text(String(localized: "button_copy_seed", defaultValue: "Copy Seed"))
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.white)
                        }
                        .frame(maxWidth: .infinity)
                        .frame(height: 48)
                        .background(Color(uiColor: .secondarySystemGroupedBackground))
                        .clipShape(Capsule())
                        .overlay(
                            Capsule().stroke(Color.white.opacity(0.08), lineWidth: 1)
                        )
                    }

                    // Right: Hide Button
                    Button {
                        let generator = UIImpactFeedbackGenerator(style: .medium)
                        generator.impactOccurred()
                        withAnimation(.spring(response: 0.35, dampingFraction: 0.82)) {
                            isRevealed = false
                        }
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "eye.slash.fill")
                                .font(.system(size: 15, weight: .semibold))
                            Text("Hide")
                                .font(.subheadline.weight(.semibold))
                        }
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .frame(height: 48)
                        .background(Color(uiColor: .secondarySystemGroupedBackground))
                        .clipShape(Capsule())
                        .overlay(
                            Capsule().stroke(Color.white.opacity(0.08), lineWidth: 1)
                        )
                    }
                }
                .padding(.horizontal, 20)
                .padding(.top, 16)
                .transition(.opacity)
            }

            Spacer()

            // Dismiss Done Button
            Button(action: onDone) {
                Text(String(localized: "button_done", defaultValue: "Done"))
                    .font(.headline)
                    .foregroundStyle(.black)
                    .frame(maxWidth: .infinity)
                    .frame(height: 52)
                    .background(Color.white)
                    .clipShape(Capsule())
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
        }
        .alert(
            "Copy Secret Recovery Phrase?",
            isPresented: $showCopyWarning
        ) {
            Button("Copy to Clipboard") {
                copySeedToClipboard()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Your clipboard is accessible to other apps. For your protection, it will be automatically cleared in 60 seconds."
            )
        }
        .onDisappear {
            cancelClipboardTasks()
        }
    }
}
