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

// MARK: - Secret Phrase Revealed View

struct RevealQuizSecretPhraseView: View {
    let mnemonic: String
    let onDone: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            ScrollView {
                VStack(spacing: 20) {
                    // Security Warning Banner
                    HStack(spacing: 10) {
                        Image(systemName: "lock.shield.fill")
                            .font(.title2)
                            .foregroundStyle(Color.stablePrimary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Secret Recovery Phrase")
                                .font(.headline)
                                .foregroundStyle(.primary)
                            Text("Keep these words strictly confidential. Never share them with anyone.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(14)
                    .background(Color(uiColor: .secondarySystemGroupedBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                    .overlay(
                        RoundedRectangle(cornerRadius: 12)
                            .stroke(Color.white.opacity(0.06), lineWidth: 1)
                    )
                    .padding(.horizontal, 20)
                    .padding(.top, 12)

                    // Seed Display Component
                    SeedDisplayView(words: mnemonic)
                        .padding(.horizontal, 20)
                }
            }

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
    }
}

// MARK: - Learn More Sheet

struct RevealQuizLearnMoreSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("Self-Custodial Security", systemImage: "shield.lefthalf.filled")
                            .font(.title2.bold())
                            .foregroundStyle(Color.stablePrimary)

                        Text(
                            "Stable Channels is 100% self-custodial. You have full ownership and control over your Bitcoin and Lightning keys."
                        )
                        .font(.body)
                        .foregroundStyle(.secondary)
                    }

                    Divider()

                    VStack(alignment: .leading, spacing: 16) {
                        securityRuleRow(
                            number: "1",
                            title: "Your Phrase Is Your Wallet",
                            desc: "The 12 or 24 words represent the master mathematical private key to all your funds."
                        )

                        securityRuleRow(
                            number: "2",
                            title: "No Support Can Recover It",
                            desc: "Because there is no central server holding your keys, if you lose your phrase, no one can restore your wallet."
                        )

                        securityRuleRow(
                            number: "3",
                            title: "Beware of Impersonators",
                            desc: "Support staff, developers, or websites asking for your recovery phrase are scammers attempting to drain your wallet."
                        )

                        securityRuleRow(
                            number: "4",
                            title: "Safe Storage Practices",
                            desc: "Store your recovery phrase physically on paper or stamped steel in a secure location. Avoid saving it in unencrypted notes or screenshots."
                        )
                    }
                }
                .padding(24)
            }
            .navigationTitle("Security Guidelines")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Close") {
                        dismiss()
                    }
                    .fontWeight(.semibold)
                }
            }
        }
        .presentationDetents([.fraction(0.85), .large])
        .presentationDragIndicator(.visible)
    }

    private func securityRuleRow(number: String, title: String, desc: String) -> some View {
        HStack(alignment: .top, spacing: 14) {
            ZStack {
                Circle()
                    .fill(Color.stablePrimary.opacity(0.15))
                    .frame(width: 32, height: 32)
                Text(number)
                    .font(.subheadline.bold())
                    .foregroundStyle(Color.stablePrimary)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.headline)
                    .foregroundStyle(.primary)
                Text(desc)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
    }
}
