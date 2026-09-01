import SwiftUI

// MARK: - Editorial Vault Illustration

struct RevealQuizPortalIllustration: View {
    @State private var isPulsing = false

    var body: some View {
        ZStack {
            // Radial spotlight / stipple halo
            Circle()
                .fill(
                    RadialGradient(
                        colors: [
                            Color.orange.opacity(0.32),
                            Color.stablePrimary.opacity(0.18),
                            Color.clear
                        ],
                        center: .center,
                        startRadius: 25,
                        endRadius: 120
                    )
                )
                .frame(width: 240, height: 240)
                .scaleEffect(isPulsing ? 1.05 : 0.95)
                .animation(
                    .easeInOut(duration: 3.2).repeatForever(autoreverses: true),
                    value: isPulsing
                )

            // Radial Stippled Line Beams (Screen-print vector lines)
            ForEach(0..<12) { i in
                Rectangle()
                    .fill(
                        LinearGradient(
                            colors: [
                                Color.orange.opacity(0.35),
                                Color.stablePrimary.opacity(0.15),
                                Color.clear
                            ],
                            startPoint: .center,
                            endPoint: .trailing
                        )
                    )
                    .frame(width: 160, height: 1.5)
                    .rotationEffect(.degrees(Double(i) * 30))
            }

            // Outer Arch Frame (Cryptographic Vault Monolith)
            ZStack {
                // Vault background fill
                UnevenRoundedRectangle(
                    topLeadingRadius: 48,
                    bottomLeadingRadius: 6,
                    bottomTrailingRadius: 6,
                    topTrailingRadius: 48,
                    style: .continuous
                )
                .fill(
                    LinearGradient(
                        colors: [
                            Color(red: 0.12, green: 0.14, blue: 0.2),
                            Color(red: 0.06, green: 0.08, blue: 0.12)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
                .frame(width: 104, height: 140)
                .overlay(
                    // Sharp dual-tone border (Bitcoin Orange to Stable Cyan)
                    UnevenRoundedRectangle(
                        topLeadingRadius: 48,
                        bottomLeadingRadius: 6,
                        bottomTrailingRadius: 6,
                        topTrailingRadius: 48,
                        style: .continuous
                    )
                    .stroke(
                        LinearGradient(
                            colors: [
                                Color.orange,
                                Color.orange.opacity(0.7),
                                Color.stablePrimary
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        lineWidth: 2.5
                    )
                )
                .shadow(color: Color.orange.opacity(0.25), radius: 14, x: 0, y: 6)

                // Inner Keyhole & Bitcoin ₿ Seal
                VStack(spacing: 8) {
                    ZStack {
                        Circle()
                            .fill(Color.orange.opacity(0.18))
                            .frame(width: 42, height: 42)

                        Text("₿")
                            .font(.system(size: 26, weight: .bold, design: .rounded))
                            .foregroundStyle(Color.orange)
                    }

                    // Keyhole silhouette
                    VStack(spacing: 0) {
                        Circle()
                            .fill(Color.white.opacity(0.85))
                            .frame(width: 10, height: 10)
                        UnevenRoundedRectangle(
                            topLeadingRadius: 0,
                            bottomLeadingRadius: 2,
                            bottomTrailingRadius: 2,
                            topTrailingRadius: 0
                        )
                        .fill(Color.white.opacity(0.85))
                        .frame(width: 6, height: 12)
                    }
                }
                .offset(y: -4)
            }
            .offset(y: -10)

            // Floating Cryptographic Badges (Minimalist Geometric Accents)
            Group {
                Image(systemName: "lock.shield.fill")
                    .font(.system(size: 15))
                    .foregroundStyle(Color.stablePrimary.opacity(0.85))
                    .offset(x: -64, y: -44)

                Image(systemName: "key.fill")
                    .font(.system(size: 14))
                    .foregroundStyle(Color.orange.opacity(0.85))
                    .offset(x: 64, y: -36)

                Image(systemName: "sparkle")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.75))
                    .offset(x: -54, y: 32)

                Image(systemName: "circle.grid.2x2.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(Color.stablePrimary.opacity(0.75))
                    .offset(x: 58, y: 28)
            }

            // Slate Floor Base
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [
                            Color(white: 0.18),
                            Color(white: 0.08)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
                .frame(width: 160, height: 14)
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(Color.white.opacity(0.12), lineWidth: 1)
                )
                .offset(y: 64)
                .shadow(color: .black.opacity(0.4), radius: 8, x: 0, y: 4)
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
        UIPasteboard.general.string = mnemonic
        let generator = UINotificationFeedbackGenerator()
        generator.notificationOccurred(.success)
        withAnimation { copiedSeed = true }

        clipboardClearTask = Task {
            try? await Task.sleep(for: .seconds(SeedConstants.clipboardClearSeconds))
            if UIPasteboard.general.string == mnemonic {
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

            // Below Grid Actions (Copy to Clipboard & Hide Toggle)
            if isRevealed {
                VStack(spacing: 8) {
                    if !copiedSeed && !showCopyWarning {
                        Button {
                            showCopyWarning = true
                        } label: {
                            HStack(spacing: 6) {
                                Image(systemName: "doc.on.doc")
                                    .font(.subheadline)
                                Text(String(localized: "button_copy_seed", defaultValue: "Copy to clipboard"))
                                    .font(.subheadline.weight(.medium))
                            }
                            .foregroundStyle(.white)
                            .padding(.vertical, 10)
                            .padding(.horizontal, 18)
                            .background(Color(uiColor: .secondarySystemGroupedBackground))
                            .clipShape(Capsule())
                            .overlay(
                                Capsule().stroke(Color.white.opacity(0.08), lineWidth: 1)
                            )
                        }
                        .padding(.top, 14)
                    }

                    if copiedSeed {
                        HStack(spacing: 6) {
                            Image(systemName: "checkmark")
                            Text(String(localized: "button_copied", defaultValue: "Copied"))
                        }
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.green)
                        .padding(.vertical, 10)
                        .padding(.horizontal, 18)
                        .background(Color.green.opacity(0.12))
                        .clipShape(Capsule())
                        .padding(.top, 14)
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
                        .background(Color(uiColor: .tertiarySystemGroupedBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                        .padding(.top, 8)
                    }

                    Button {
                        let generator = UIImpactFeedbackGenerator(style: .light)
                        generator.impactOccurred()
                        withAnimation(.spring(response: 0.35, dampingFraction: 0.82)) {
                            isRevealed = false
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "eye.slash.fill")
                            Text("Hide phrase")
                        }
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                    }
                    .padding(.top, 6)
                }
                .padding(.horizontal, 20)
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
        .onDisappear {
            cancelClipboardTasks()
        }
    }
}

// MARK: - Learn More Sheet (Editorial Luxury Design)

struct RevealQuizLearnMoreSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()

                ScrollView(showsIndicators: false) {
                    VStack(spacing: 24) {
                        // Hero Header with Glowing Shield Badge
                        VStack(spacing: 14) {
                            ZStack {
                                Circle()
                                    .fill(
                                        RadialGradient(
                                            colors: [Color.stablePrimary.opacity(0.25), Color.clear],
                                            center: .center,
                                            startRadius: 8,
                                            endRadius: 44
                                        )
                                    )
                                    .frame(width: 88, height: 88)

                                Circle()
                                    .fill(Color(uiColor: .secondarySystemGroupedBackground))
                                    .frame(width: 58, height: 58)
                                    .overlay(
                                        Circle()
                                            .stroke(Color.white.opacity(0.08), lineWidth: 1)
                                    )

                                Image(systemName: "shield.checkered")
                                    .font(.system(size: 26, weight: .semibold))
                                    .foregroundStyle(Color.stablePrimary)
                            }
                            .padding(.top, 8)

                            VStack(spacing: 6) {
                                Text("Self-Custody Guidelines")
                                    .font(.title2.weight(.bold))
                                    .foregroundStyle(.white)

                                Text(
                                    "Your Secret Recovery Phrase is the cryptographic root of your entire wallet. Keep these rules top of mind."
                                )
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.center)
                                .padding(.horizontal, 20)
                                .lineSpacing(3)
                            }
                        }
                        .padding(.horizontal, 16)

                        // 4 Luxury Security Pillar Cards
                        VStack(spacing: 12) {
                            luxuryGuidelineCard(
                                icon: "key.fill",
                                iconColor: Color.stablePrimary,
                                number: "01",
                                title: "Your Master Cryptographic Key",
                                desc: "Your recovery words mathematically derive all private keys. Anyone with these 12 words has full, irreversible ownership of your funds."
                            )

                            luxuryGuidelineCard(
                                icon: "server.rack",
                                iconColor: Color.orange,
                                number: "02",
                                title: "Zero Server Backups",
                                desc: "Stable Channels is 100% self-custodial. We never store or transmit your keys. If you lose your phrase, no one in the world can restore it."
                            )

                            luxuryGuidelineCard(
                                icon: "exclamationmark.shield.fill",
                                iconColor: Color.red,
                                number: "03",
                                title: "Beware of Impersonators",
                                desc: "No support agent, team member, or bot will EVER ask for your phrase. Anyone asking for your words is an active scammer."
                            )

                            luxuryGuidelineCard(
                                icon: "lock.square.stack.fill",
                                iconColor: Color.cyan,
                                number: "04",
                                title: "Store Offline on Physical Media",
                                desc: "Engrave on steel or write on paper kept in a secure vault. Never take screenshots, upload to cloud storage, or paste into digital notes."
                            )
                        }
                        .padding(.horizontal, 20)

                        // Bottom Understood Action Button
                        Button {
                            dismiss()
                        } label: {
                            Text("I Understand")
                                .font(.headline)
                                .foregroundStyle(.black)
                                .frame(maxWidth: .infinity)
                                .frame(height: 52)
                                .background(Color.white)
                                .clipShape(Capsule())
                        }
                        .padding(.horizontal, 24)
                        .padding(.top, 8)
                        .padding(.bottom, 24)
                    }
                    .padding(.top, 12)
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

    private func luxuryGuidelineCard(
        icon: String,
        iconColor: Color,
        number: String,
        title: String,
        desc: String
    ) -> some View {
        HStack(alignment: .top, spacing: 14) {
            // Icon Badge
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(iconColor.opacity(0.12))
                    .frame(width: 44, height: 44)
                    .overlay(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .stroke(iconColor.opacity(0.24), lineWidth: 1)
                    )

                Image(systemName: icon)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(iconColor)
            }

            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(title)
                        .font(.headline)
                        .foregroundStyle(.white)

                    Spacer()

                    Text(number)
                        .font(.caption2.monospaced().bold())
                        .foregroundStyle(iconColor.opacity(0.8))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(iconColor.opacity(0.1))
                        .clipShape(Capsule())
                }

                Text(desc)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineSpacing(2)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.white.opacity(0.06), lineWidth: 1)
        )
    }
}
