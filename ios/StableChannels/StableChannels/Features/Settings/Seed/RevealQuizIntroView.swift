import SwiftUI

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
