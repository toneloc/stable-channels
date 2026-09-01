import SwiftUI

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
