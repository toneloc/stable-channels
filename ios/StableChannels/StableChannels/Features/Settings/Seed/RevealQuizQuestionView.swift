import SwiftUI

// MARK: - Question Screen (Step 1 & 2)

struct RevealQuizQuestionView: View {
    let question: RevealQuizQuestion
    let onSelectOption: (Int) -> Void
    let onLearnMore: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    // Header indicator
                    Text("Question \(question.questionNumber) of \(question.totalQuestions)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .padding(.top, 16)

                    // Question prompt
                    Text(question.prompt)
                        .font(.title.bold())
                        .foregroundStyle(.primary)
                        .padding(.top, 12)
                        .lineSpacing(2)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 24)
            }
            .scrollBounceBehavior(.basedOnSize)

            // Option selection pills
            VStack(spacing: 12) {
                ForEach(Array(question.options.enumerated()), id: \.offset) { index, option in
                    Button {
                        onSelectOption(index)
                    } label: {
                        Text(option)
                            .font(.body.weight(.medium))
                            .foregroundStyle(.primary)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 16)
                            .padding(.vertical, 8)
                            .frame(maxWidth: .infinity, minHeight: 54)
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
                        .frame(minHeight: 44)
                        .contentShape(Rectangle())
                }
                .padding(.top, 8)
                .padding(.bottom, 8)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 20)
            .layoutPriority(1)
        }
    }
}
