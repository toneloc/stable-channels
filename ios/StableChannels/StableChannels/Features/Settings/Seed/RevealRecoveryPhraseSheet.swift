import SwiftUI

/// Master sheet container hosting the Reveal Secret Recovery Phrase quiz flow.
struct RevealRecoveryPhraseSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var viewModel: RevealRecoveryQuizViewModel

    init(
        mnemonic: String,
        questionsProvider: any RevealQuizQuestionsProviderProtocol = DefaultRevealQuizQuestionsProvider()
    ) {
        _viewModel = State(initialValue: RevealRecoveryQuizViewModel(
            mnemonic: mnemonic,
            questionsProvider: questionsProvider
        ))
    }

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()

                VStack(spacing: 0) {
                    // Content step views
                    switch viewModel.currentStep {
                    case .intro:
                        RevealQuizIntroView(
                            onGetStarted: {
                                withAnimation(.easeInOut(duration: 0.25)) {
                                    viewModel.startQuiz()
                                }
                            },
                            onLearnMore: {
                                viewModel.isShowingLearnMore = true
                            }
                        )
                        .transition(stepTransition)

                    case let .question(index):
                        if index < viewModel.questions.count {
                            RevealQuizQuestionView(
                                question: viewModel.questions[index],
                                onSelectOption: { optionIndex in
                                    withAnimation(.easeInOut(duration: 0.25)) {
                                        viewModel.selectOption(optionIndex)
                                    }
                                },
                                onLearnMore: {
                                    viewModel.isShowingLearnMore = true
                                }
                            )
                            .transition(stepTransition)
                        }

                    case let .feedback(questionIndex, isCorrect, _):
                        if questionIndex < viewModel.questions.count {
                            RevealQuizFeedbackView(
                                question: viewModel.questions[questionIndex],
                                isCorrect: isCorrect,
                                onContinue: {
                                    withAnimation(.easeInOut(duration: 0.25)) {
                                        viewModel.continueFromFeedback()
                                    }
                                },
                                onLearnMore: {
                                    viewModel.isShowingLearnMore = true
                                }
                            )
                            .transition(stepTransition)
                        }

                    case .revealed:
                        RevealQuizSecretPhraseView(
                            mnemonic: viewModel.mnemonic,
                            onDone: {
                                dismiss()
                            }
                        )
                        .transition(stepTransition)
                    }
                }
            }
            .navigationTitle(viewModel.navigationTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(.white)
                    }
                }
            }
            .sheet(isPresented: $viewModel.isShowingLearnMore) {
                RevealQuizLearnMoreSheet()
            }
        }
        .preferredColorScheme(.dark)
        .interactiveDismissDisabled(true)
    }

    private var stepTransition: AnyTransition {
        if viewModel.navigationDirection == .forward {
            return .asymmetric(
                insertion: .opacity.combined(with: .move(edge: .trailing)),
                removal: .opacity.combined(with: .move(edge: .leading))
            )
        } else {
            return .asymmetric(
                insertion: .opacity.combined(with: .move(edge: .leading)),
                removal: .opacity.combined(with: .move(edge: .trailing))
            )
        }
    }
}
