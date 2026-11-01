import Foundation
import Observation
import SwiftUI

enum QuizNavigationDirection: Equatable {
    case forward
    case backward
}

/// State machine and view model for the recovery phrase reveal quiz.
@Observable
@MainActor
final class RevealRecoveryQuizViewModel {
    var currentStep: RevealQuizStep = .intro
    var navigationDirection: QuizNavigationDirection = .forward
    var isShowingLearnMore: Bool = false
    let mnemonic: String
    let questions: [RevealQuizQuestion]

    init(
        mnemonic: String,
        questionsProvider: any RevealQuizQuestionsProviderProtocol = DefaultRevealQuizQuestionsProvider()
    ) {
        self.mnemonic = mnemonic
        self.questions = questionsProvider.getQuestions()
    }

    // MARK: - Navigation State Queries

    var canGoBack: Bool {
        switch currentStep {
        case .intro, .revealed:
            return false
        case .question, .feedback:
            return true
        }
    }

    var navigationTitle: String {
        String(localized: "reveal_phrase_nav_title", defaultValue: "Reveal Secret Recovery Phrase")
    }

    /// Leaving the app closes the reveal, because the cover sits above the app lock overlay.
    /// Not `.inactive`: the Face ID prompt raises it, and the phrase view re-blurs there already.
    static func shouldDismiss(for phase: ScenePhase) -> Bool {
        phase == .background
    }

    // MARK: - Actions

    func startQuiz() {
        navigationDirection = .forward
        guard !questions.isEmpty else {
            currentStep = .revealed
            return
        }
        currentStep = .question(index: 0)
    }

    /// Answers the question currently on screen; ignored outside a question step.
    func selectOption(_ optionIndex: Int) {
        guard case let .question(questionIndex) = currentStep, questions.indices.contains(questionIndex) else {
            return
        }
        navigationDirection = .forward
        let isCorrect = questions[questionIndex].isCorrect(optionIndex: optionIndex)
        currentStep = .feedback(questionIndex: questionIndex, isCorrect: isCorrect, selectedOption: optionIndex)
    }

    /// Advances from feedback: next question or reveal when correct, retry when wrong.
    func continueFromFeedback() {
        guard case let .feedback(questionIndex, isCorrect, _) = currentStep else { return }
        navigationDirection = .forward
        guard isCorrect else {
            currentStep = .question(index: questionIndex)
            return
        }
        let nextIndex = questionIndex + 1
        currentStep = questions.indices.contains(nextIndex) ? .question(index: nextIndex) : .revealed
    }

    func goBack() {
        navigationDirection = .backward
        switch currentStep {
        case .intro:
            break
        case let .question(index):
            if index > 0 {
                currentStep = .question(index: index - 1)
            } else {
                currentStep = .intro
            }
        case let .feedback(questionIndex, _, _):
            currentStep = .question(index: questionIndex)
        case .revealed:
            break
        }
    }
}
