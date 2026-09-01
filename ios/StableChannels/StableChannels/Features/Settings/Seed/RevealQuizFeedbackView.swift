import SwiftUI

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
