import Foundation
import Observation

enum QuizNavigationDirection: Equatable {
    case forward
    case backward
}

/// State machine and view model for the recovery phrase reveal quiz.
@Observable
final class RevealRecoveryQuizViewModel {
    private let questionsProvider: any RevealQuizQuestionsProviderProtocol

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
        self.questionsProvider = questionsProvider
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

    // MARK: - Actions

    func startQuiz() {
        navigationDirection = .forward
        guard !questions.isEmpty else {
            currentStep = .revealed
            return
        }
        currentStep = .question(index: 0)
    }

    func answerQuestion(questionIndex: Int, optionIndex: Int) {
        navigationDirection = .forward
        guard questionIndex >= 0, questionIndex < questions.count else { return }
        let question = questions[questionIndex]
        let isCorrect = question.isCorrect(optionIndex: optionIndex)
        currentStep = .feedback(questionIndex: questionIndex, isCorrect: isCorrect, selectedOption: optionIndex)
    }

    func continueFromFeedback(questionIndex: Int, isCorrect: Bool) {
        navigationDirection = .forward
        if isCorrect {
            let nextIndex = questionIndex + 1
            if nextIndex < questions.count {
                currentStep = .question(index: nextIndex)
            } else {
                currentStep = .revealed
            }
        } else {
            // Retry the current question
            currentStep = .question(index: questionIndex)
        }
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
