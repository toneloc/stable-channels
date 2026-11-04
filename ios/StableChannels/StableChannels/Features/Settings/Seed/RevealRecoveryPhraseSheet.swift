import SwiftUI

/// Master sheet container hosting the Reveal Secret Recovery Phrase quiz flow.
struct RevealRecoveryPhraseSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var viewModel: RevealRecoveryQuizViewModel

    init(mnemonicProvider: @escaping () -> String?) {
        _viewModel = State(initialValue: RevealRecoveryQuizViewModel(mnemonicProvider: mnemonicProvider))
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
                                withAnimation(Self.stepAnimation) {
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
                                progress: viewModel.progressText(forQuestionAt: index),
                                onSelectOption: { optionIndex in
                                    withAnimation(Self.stepAnimation) {
                                        viewModel.selectOption(optionIndex)
                                    }
                                },
                                onLearnMore: {
                                    viewModel.isShowingLearnMore = true
                                }
                            )
                            .transition(stepTransition)
                        }

                    case let .feedback(questionIndex, isCorrect):
                        if questionIndex < viewModel.questions.count {
                            RevealQuizFeedbackView(
                                question: viewModel.questions[questionIndex],
                                progress: viewModel.progressText(forQuestionAt: questionIndex),
                                isCorrect: isCorrect,
                                onContinue: {
                                    withAnimation(Self.stepAnimation) {
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
                        if let mnemonic = viewModel.mnemonic {
                            RevealQuizSecretPhraseView(
                                mnemonic: mnemonic,
                                onDone: {
                                    dismiss()
                                }
                            )
                            .transition(stepTransition)
                        } else {
                            // The phrase was removed during the quiz: close rather than show an empty screen.
                            Color.clear.onAppear { dismiss() }
                        }
                    }
                }
            }
            .navigationTitle(viewModel.navigationTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        if viewModel.canGoBack {
                            withAnimation(Self.stepAnimation) {
                                viewModel.goBack()
                            }
                        } else {
                            dismiss()
                        }
                    } label: {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(.white)
                    }
                    .accessibilityLabel(viewModel.canGoBack
                        ? String(localized: "button_back", defaultValue: "Back")
                        : String(localized: "button_close", defaultValue: "Close"))
                }
            }
            .sheet(isPresented: $viewModel.isShowingLearnMore) {
                RevealQuizLearnMoreSheet()
            }
        }
        .preferredColorScheme(.dark)
        .interactiveDismissDisabled(true)
    }

    private static let stepAnimation = Animation.easeInOut(duration: 0.25)

    /// Removal stays a fade: the outgoing view keeps the transition from its last render, before the direction changed.
    private var stepTransition: AnyTransition {
        guard !reduceMotion else { return .opacity }
        let insertionEdge: Edge = viewModel.navigationDirection == .forward ? .trailing : .leading
        return .asymmetric(
            insertion: .opacity.combined(with: .move(edge: insertionEdge)),
            removal: .opacity
        )
    }
}
