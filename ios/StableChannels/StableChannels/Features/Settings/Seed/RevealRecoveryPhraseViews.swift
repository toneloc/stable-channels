import SwiftUI

// MARK: - Locked Vault Door Illustration

struct RevealQuizPortalIllustration: View {
    @State private var isPulsing = false

    var body: some View {
        ZStack {
            // Ambient Radial Spotlight / Cryptographic Halo
            Circle()
                .fill(
                    RadialGradient(
                        colors: [
                            Color.orange.opacity(0.30),
                            Color.stablePrimary.opacity(0.16),
                            Color.clear
                        ],
                        center: .center,
                        startRadius: 20,
                        endRadius: 120
                    )
                )
                .frame(width: 240, height: 240)
                .scaleEffect(isPulsing ? 1.06 : 0.94)
                .animation(
                    .easeInOut(duration: 3.2).repeatForever(autoreverses: true),
                    value: isPulsing
                )

            // Radial Geometric Ray Beams
            ForEach(0..<8) { i in
                Rectangle()
                    .fill(
                        LinearGradient(
                            colors: [
                                Color.orange.opacity(0.30),
                                Color.stablePrimary.opacity(0.12),
                                Color.clear
                            ],
                            startPoint: .center,
                            endPoint: .trailing
                        )
                    )
                    .frame(width: 150, height: 1.5)
                    .rotationEffect(.degrees(Double(i) * 45))
            }

            // Locked Security Vault Door Assembly
            ZStack {
                // Outer Heavy Steel Door Frame
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(
                        LinearGradient(
                            colors: [
                                Color(white: 0.18),
                                Color(white: 0.08)
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        )
                    )
                    .frame(width: 124, height: 156)
                    .overlay(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .stroke(
                                LinearGradient(
                                    colors: [
                                        Color.orange.opacity(0.8),
                                        Color.stablePrimary.opacity(0.7)
                                    ],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                ),
                                lineWidth: 2
                            )
                    )
                    .shadow(color: Color.orange.opacity(0.2), radius: 14, x: 0, y: 6)

                // Inner Vault Door Leaf
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(
                        LinearGradient(
                            colors: [
                                Color(white: 0.13),
                                Color(white: 0.05)
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    )
                    .frame(width: 108, height: 140)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(Color.white.opacity(0.08), lineWidth: 1)
                    )

                // Top & Bottom Inset Steel Panels
                VStack(spacing: 16) {
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(Color.black.opacity(0.4))
                        .frame(width: 88, height: 42)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .stroke(Color.white.opacity(0.05), lineWidth: 1)
                        )

                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(Color.black.opacity(0.4))
                        .frame(width: 88, height: 42)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .stroke(Color.white.opacity(0.05), lineWidth: 1)
                        )
                }

                // Steel Door Hinges (Left side)
                VStack(spacing: 48) {
                    RoundedRectangle(cornerRadius: 2)
                        .fill(Color.orange.opacity(0.8))
                        .frame(width: 6, height: 16)
                    RoundedRectangle(cornerRadius: 2)
                        .fill(Color.orange.opacity(0.8))
                        .frame(width: 6, height: 16)
                }
                .offset(x: -58)

                // Central Heavy Lock & ₿ Emblem
                ZStack {
                    // Lock Base Glow Plate
                    Circle()
                        .fill(
                            LinearGradient(
                                colors: [
                                    Color.orange.opacity(0.25),
                                    Color.black
                                ],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                        )
                        .frame(width: 48, height: 48)
                        .overlay(
                            Circle()
                                .stroke(Color.orange.opacity(0.6), lineWidth: 1.5)
                        )

                    // Padlock Silhouette & ₿ Symbol
                    VStack(spacing: 2) {
                        Image(systemName: "lock.fill")
                            .font(.system(size: 22, weight: .bold))
                            .foregroundStyle(
                                LinearGradient(
                                    colors: [
                                        Color.orange,
                                        Color(red: 1.0, green: 0.72, blue: 0.3)
                                    ],
                                    startPoint: .top,
                                    endPoint: .bottom
                                )
                            )
                            .shadow(color: Color.orange.opacity(0.5), radius: 6, x: 0, y: 2)

                        Text("₿")
                            .font(.system(size: 11, weight: .black, design: .rounded))
                            .foregroundStyle(.black)
                            .offset(y: -14)
                    }
                }
            }
            .offset(y: -10)

            // Floating Key & Security Accents
            Group {
                Image(systemName: "key.fill")
                    .font(.system(size: 16))
                    .foregroundStyle(Color.orange.opacity(0.85))
                    .rotationEffect(.degrees(-35))
                    .offset(x: 68, y: -38)

                Image(systemName: "sparkle")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.8))
                    .offset(x: -66, y: -42)

                Image(systemName: "shield.fill")
                    .font(.system(size: 13))
                    .foregroundStyle(Color.stablePrimary.opacity(0.8))
                    .offset(x: -64, y: 32)
            }

            // Heavy Steel Platform Base
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [
                            Color(white: 0.22),
                            Color(white: 0.08)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
                .frame(width: 164, height: 12)
                .overlay(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .stroke(Color.white.opacity(0.12), lineWidth: 1)
                )
                .offset(y: 72)
                .shadow(color: .black.opacity(0.5), radius: 8, x: 0, y: 4)
        }
        .frame(height: 200)
        .onAppear {
            isPulsing = true
        }
    }
}

// MARK: - Intro Screen (Step 0)

struct RevealQuizIntroView: View {
    let onGetStarted: () -> Void
    let onLearnMore: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Spacer()

            // Editorial Vault Artwork
            RevealQuizPortalIllustration()
                .padding(.bottom, 36)

            // Subtitle Description
            Text(String(
                localized: "reveal_quiz_intro_desc",
                defaultValue: "To reveal your Secret Recovery Phrase, you need to correctly answer two questions"
            ))
            .font(.body)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 32)
            .lineSpacing(4)

            Spacer()

            // Bottom Actions
            VStack(spacing: 16) {
                Button(action: onGetStarted) {
                    Text(String(localized: "reveal_quiz_get_started", defaultValue: "Get started"))
                        .font(.headline)
                        .foregroundStyle(.black)
                        .frame(maxWidth: .infinity)
                        .frame(height: 52)
                        .background(Color.white)
                        .clipShape(Capsule())
                }

                Button(action: onLearnMore) {
                    Text(String(localized: "reveal_quiz_learn_more", defaultValue: "Learn more"))
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(Color.stablePrimary)
                }
                .padding(.bottom, 8)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
        }
    }
}

// MARK: - Question Screen (Step 1 & 2)

struct RevealQuizQuestionView: View {
    let question: RevealQuizQuestion
    let onSelectOption: (Int) -> Void
    let onLearnMore: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Header indicator
            Text("Question \(question.questionNumber) of \(question.totalQuestions)")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 24)
                .padding(.top, 16)

            // Question prompt
            Text(question.prompt)
                .font(.system(size: 26, weight: .bold))
                .foregroundStyle(.primary)
                .padding(.horizontal, 24)
                .padding(.top, 12)
                .lineSpacing(2)

            Spacer()

            // Option selection pills
            VStack(spacing: 12) {
                ForEach(Array(question.options.enumerated()), id: \.offset) { index, option in
                    Button {
                        onSelectOption(index)
                    } label: {
                        Text(option)
                            .font(.body.weight(.medium))
                            .foregroundStyle(.primary)
                            .frame(maxWidth: .infinity)
                            .frame(height: 54)
                            .background(Color(uiColor: .secondarySystemGroupedBackground))
                            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: 14, style: .continuous)
                                    .stroke(Color.white.opacity(0.06), lineWidth: 1)
                            )
                    }
                    .buttonStyle(.plain)
                }

                Button(action: onLearnMore) {
                    Text(String(localized: "reveal_quiz_learn_more", defaultValue: "Learn more"))
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(Color.stablePrimary)
                }
                .padding(.top, 8)
                .padding(.bottom, 8)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
        }
    }
}

// MARK: - Feedback Screen (Correct / Incorrect)

struct RevealQuizFeedbackView: View {
    let question: RevealQuizQuestion
    let isCorrect: Bool
    let onContinue: () -> Void
    let onLearnMore: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Header indicator
            Text("Question \(question.questionNumber) of \(question.totalQuestions)")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 24)
                .padding(.top, 16)

            // Result status badge
            HStack(spacing: 8) {
                Image(systemName: isCorrect ? "checkmark.circle.fill" : "xmark.circle.fill")
                    .font(.title3.bold())
                Text(isCorrect ? "Correct!" : "Incorrect!")
                    .font(.title3.bold())
            }
            .foregroundStyle(isCorrect ? Color.green : Color.red)
            .padding(.horizontal, 24)
            .padding(.top, 16)

            // Result heading
            Text(isCorrect ? question.correctHeading : question.incorrectHeading)
                .font(.system(size: 24, weight: .bold))
                .foregroundStyle(.primary)
                .padding(.horizontal, 24)
                .padding(.top, 12)
                .lineSpacing(2)

            // Result detailed explanation
            Text(isCorrect ? question.correctExplanation : question.incorrectExplanation)
                .font(.body)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 24)
                .padding(.top, 16)
                .lineSpacing(4)

            Spacer()

            // Bottom action button
            VStack(spacing: 16) {
                Button(action: onContinue) {
                    Text(
                        isCorrect
                            ? (question.isLastQuestion ? String(
                                localized: "button_reveal_phrase",
                                defaultValue: "Continue"
                            ) : String(localized: "button_continue", defaultValue: "Continue"))
                            : String(localized: "button_try_again", defaultValue: "Try again")
                    )
                    .font(.headline)
                    .foregroundStyle(.black)
                    .frame(maxWidth: .infinity)
                    .frame(height: 52)
                    .background(Color.white)
                    .clipShape(Capsule())
                }

                Button(action: onLearnMore) {
                    Text(String(localized: "reveal_quiz_learn_more", defaultValue: "Learn more"))
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(Color.stablePrimary)
                }
                .padding(.bottom, 8)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
        }
    }
}

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

// MARK: - Learn More Sheet

struct RevealQuizLearnMoreSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()

                VStack(spacing: 0) {
                    ScrollView(showsIndicators: false) {
                        VStack(alignment: .leading, spacing: 32) {
                            // Hero Title Header
                            VStack(alignment: .leading, spacing: 12) {
                                Image(systemName: "shield.checkered")
                                    .font(.system(size: 40))
                                    .foregroundStyle(Color.stablePrimary)

                                Text("Self-Custody Guidelines")
                                    .font(.system(size: 28, weight: .bold, design: .default))
                                    .foregroundStyle(.white)

                                Text(
                                    "Your Secret Recovery Phrase gives complete ownership of your Bitcoin and Lightning funds. Follow these core principles to stay secure."
                                )
                                .font(.body)
                                .foregroundStyle(.secondary)
                                .lineSpacing(4)
                            }
                            .padding(.top, 16)

                            // Apple HIG Feature List
                            VStack(spacing: 24) {
                                appleGuidelineRow(
                                    icon: "key.horizontal.fill",
                                    title: "Your Words Are Your Master Key",
                                    desc: "The 12 words mathematically derive all private keys in your wallet. Anyone with these words has full, irreversible control of your funds."
                                )

                                appleGuidelineRow(
                                    icon: "icloud.slash",
                                    title: "No Server Backups",
                                    desc: "Stable Channels is non-custodial. We never store or transmit your keys. If you lose your phrase, no one can recover your wallet."
                                )

                                appleGuidelineRow(
                                    icon: "exclamationmark.shield.fill",
                                    title: "Beware of Impersonators",
                                    desc: "Support staff, developers, or bots will never ask for your recovery phrase. Never share it with anyone under any circumstance."
                                )

                                appleGuidelineRow(
                                    icon: "lock.shield.fill",
                                    title: "Store Offline on Physical Media",
                                    desc: "Write your words on paper or stamp them on metal kept in a private, secure location. Never take screenshots or save digital copies in cloud notes."
                                )
                            }
                        }
                        .padding(.horizontal, 28)
                        .padding(.bottom, 24)
                    }

                    // Bottom Action Button
                    Button {
                        dismiss()
                    } label: {
                        Text("Done")
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
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 20))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .preferredColorScheme(.dark)
    }

    private func appleGuidelineRow(
        icon: String,
        title: String,
        desc: String
    ) -> some View {
        HStack(alignment: .top, spacing: 18) {
            Image(systemName: icon)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(Color.stablePrimary)
                .frame(width: 32, alignment: .center)

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.headline)
                    .foregroundStyle(.primary)

                Text(desc)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineSpacing(3)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
