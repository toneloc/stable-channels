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

    private let columns = [
        GridItem(.flexible(), spacing: 8),
        GridItem(.flexible(), spacing: 8),
        GridItem(.flexible(), spacing: 8)
    ]

    private func copySeedToClipboard() {
        clipboardClearTask?.cancel()
        clipboardFadeTask?.cancel()

        // Apple Native Clipboard Security: Expiration Date & Local-Only (prevents iCloud Universal Clipboard leakage)
        let expirationDate = Date().addingTimeInterval(Double(SeedConstants.clipboardClearSeconds))
        UIPasteboard.general.setItems(
            [["public.utf8-plain-text": mnemonic]],
            options: [
                .expirationDate: expirationDate,
                .localOnly: true
            ]
        )

        // In-App Fallback Timer to wipe pasteboard string
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
            ScrollView(showsIndicators: false) {
                VStack(spacing: 16) {
                    // Header Subtitle
                    Text("Your Secret Recovery Phrase gives full access to your wallet. Do not share it with anyone.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 24)
                        .padding(.top, 12)
                        .lineSpacing(3)

                    // Seed Grid Area with In-Place Blur & Tap to Reveal Overlay
                    ZStack {
                        // Background Obsidian Container
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .fill(Color(uiColor: .secondarySystemGroupedBackground))
                            .overlay(
                                RoundedRectangle(cornerRadius: 18, style: .continuous)
                                    .stroke(Color.white.opacity(0.08), lineWidth: 1)
                            )

                        // 3 Equal-Sized Boxes Grid Layout
                        LazyVGrid(columns: columns, spacing: 8) {
                            ForEach(Array(wordList.enumerated()), id: \.offset) { index, word in
                                HStack(spacing: 4) {
                                    Text("\(index + 1).")
                                        .font(.system(size: 12, weight: .bold, design: .monospaced))
                                        .foregroundStyle(Color(white: 0.5))
                                        .fixedSize()

                                    Text(isRevealed ? word : "•••••")
                                        .font(.system(size: 13, weight: .medium, design: .monospaced))
                                        .foregroundStyle(.white)
                                        .lineLimit(1)
                                        .minimumScaleFactor(0.8)
                                }
                                .padding(.horizontal, 8)
                                .frame(height: 38)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(Color.black)
                                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                                        .stroke(Color.white.opacity(0.12), lineWidth: 1)
                                )
                            }
                        }
                        .padding(14)
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
                                .background(Color.black.opacity(0.55))
                                .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                            }
                            .buttonStyle(.plain)
                            .transition(.opacity)
                        }
                    }
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
                        .padding(.top, 4)
                        .transition(.opacity)
                    }
                }
                .padding(.bottom, 24)
            }

            // Dismiss Done Button pinned to bottom
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
