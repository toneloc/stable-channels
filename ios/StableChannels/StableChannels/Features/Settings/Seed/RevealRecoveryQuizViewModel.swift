import Foundation
import Observation

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
    let questions: [RevealQuizQuestion]
    private let mnemonicProvider: () -> String?

    /// Read from its source only once the quiz is passed, so the phrase is not held during the quiz.
    var mnemonic: String? {
        guard currentStep == .revealed, let phrase = mnemonicProvider(), !phrase.isEmpty else { return nil }
        return phrase
    }

    init(mnemonicProvider: @escaping () -> String?) {
        self.mnemonicProvider = mnemonicProvider
        self.questions = RevealQuizQuestion.all
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

    /// "Question N of M" for the question at `index`, counted from the question list.
    func progressText(forQuestionAt index: Int) -> String {
        String(localized: "reveal_quiz_progress", defaultValue: "Question \(index + 1) of \(questions.count)")
    }

    // MARK: - Actions

    func startQuiz() {
        navigationDirection = .forward
        currentStep = .question(index: 0)
    }

    /// Answers the question currently on screen; ignored outside a question step.
    func selectOption(_ optionIndex: Int) {
        guard case let .question(questionIndex) = currentStep, questions.indices.contains(questionIndex) else {
            return
        }
        navigationDirection = .forward
        let isCorrect = questions[questionIndex].isCorrect(optionIndex: optionIndex)
        currentStep = .feedback(questionIndex: questionIndex, isCorrect: isCorrect)
    }

    /// Advances from feedback: next question or reveal when correct, retry when wrong.
    func continueFromFeedback() {
        guard case let .feedback(questionIndex, isCorrect) = currentStep else { return }
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
        case let .feedback(questionIndex, _):
            currentStep = .question(index: questionIndex)
        case .revealed:
            break
        }
    }
}
