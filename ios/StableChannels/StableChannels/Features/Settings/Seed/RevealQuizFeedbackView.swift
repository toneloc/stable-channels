import SwiftUI

// MARK: - Feedback Screen (Correct / Incorrect)

struct RevealQuizFeedbackView: View {
    let question: RevealQuizQuestion
    let progress: String
    let isCorrect: Bool
    let onContinue: () -> Void
    let onLearnMore: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    // Header indicator
                    Text(progress)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .padding(.top, 16)

                    // Result status badge
                    HStack(spacing: 8) {
                        Image(systemName: isCorrect ? "checkmark.circle.fill" : "xmark.circle.fill")
                            .font(.title3.bold())
                            .accessibilityHidden(true)
                        Text(isCorrect
                            ? String(localized: "reveal_quiz_correct", defaultValue: "Correct!")
                            : String(localized: "reveal_quiz_incorrect", defaultValue: "Incorrect!"))
                            .font(.title3.bold())
                    }
                    .foregroundStyle(isCorrect ? Color.green : Color.red)
                    .padding(.top, 16)

                    // Result heading
                    Text(isCorrect ? question.correctHeading : question.incorrectHeading)
                        .font(.title2.bold())
                        .foregroundStyle(.primary)
                        .padding(.top, 12)
                        .lineSpacing(2)

                    // Result detailed explanation
                    Text(isCorrect ? question.correctExplanation : question.incorrectExplanation)
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .padding(.top, 16)
                        .lineSpacing(4)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 24)
            }
            .scrollBounceBehavior(.basedOnSize)

            // Bottom action button
            VStack(spacing: 16) {
                Button(action: onContinue) {
                    Text(isCorrect
                        ? String(localized: "button_continue", defaultValue: "Continue")
                        : String(localized: "button_try_again", defaultValue: "Try again"))
                        .font(.headline)
                        .foregroundStyle(Color.revealButtonLabel)
                        .frame(maxWidth: .infinity, minHeight: 52)
                        .background(Color.revealButtonFill)
                        .clipShape(Capsule())
                }

                Button(action: onLearnMore) {
                    Text(String(localized: "reveal_quiz_learn_more", defaultValue: "Learn more"))
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(Color.stablePrimary)
                        .frame(minHeight: 44)
                        .contentShape(Rectangle())
                }
                .padding(.bottom, 8)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
            .layoutPriority(1)
        }
    }
}
