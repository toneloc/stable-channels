import Foundation

/// Domain model representing a single security verification question in the reveal phrase quiz.
struct RevealQuizQuestion: Identifiable, Equatable {
    let id: Int
    let questionNumber: Int
    let totalQuestions: Int
    let prompt: String
    let options: [String]
    let correctOptionIndex: Int
    let correctHeading: String
    let correctExplanation: String
    let incorrectHeading: String
    let incorrectExplanation: String

    var isLastQuestion: Bool {
        questionNumber == totalQuestions
    }

    func isCorrect(optionIndex: Int) -> Bool {
        optionIndex == correctOptionIndex
    }
}

/// Navigation and presentation state machine for the reveal phrase quiz flow.
enum RevealQuizStep: Equatable {
    case intro
    case question(index: Int)
    case feedback(questionIndex: Int, isCorrect: Bool, selectedOption: Int)
    case revealed

    static func == (lhs: RevealQuizStep, rhs: RevealQuizStep) -> Bool {
        switch (lhs, rhs) {
        case (.intro, .intro):
            return true
        case let (.question(lIdx), .question(rIdx)):
            return lIdx == rIdx
        case let (.feedback(lQ, lC, lO), .feedback(rQ, rC, rO)):
            return lQ == rQ && lC == rC && lO == rO
        case (.revealed, .revealed):
            return true
        default:
            return false
        }
    }
}

/// Provider protocol for quiz questions to support dependency injection and testability.
protocol RevealQuizQuestionsProviderProtocol {
    func getQuestions() -> [RevealQuizQuestion]
}

/// Default questions provider aligned with self-custody principles and phishing education.
struct DefaultRevealQuizQuestionsProvider: RevealQuizQuestionsProviderProtocol {
    func getQuestions() -> [RevealQuizQuestion] {
        [
            RevealQuizQuestion(
                id: 1,
                questionNumber: 1,
                totalQuestions: 2,
                prompt: String(
                    localized: "quiz_q1_prompt",
                    defaultValue: "If you lose your Secret Recovery Phrase, Stable Channels..."
                ),
                options: [
                    String(localized: "quiz_q1_opt1", defaultValue: "Can get it back for you"),
                    String(localized: "quiz_q1_opt2", defaultValue: "Can't help you")
                ],
                correctOptionIndex: 1,
                correctHeading: String(
                    localized: "quiz_q1_correct_heading",
                    defaultValue: "Right! No one can help get your Secret Recovery Phrase back"
                ),
                correctExplanation: String(
                    localized: "quiz_q1_correct_exp",
                    defaultValue: "Write it down, engrave it on metal, or keep it in multiple secret spots so you never lose it. If you lose it, it's gone forever."
                ),
                incorrectHeading: String(
                    localized: "quiz_q1_incorrect_heading",
                    defaultValue: "Wrong! No one can help get your Secret Recovery Phrase back"
                ),
                incorrectExplanation: String(
                    localized: "quiz_q1_incorrect_exp",
                    defaultValue: "If you lose your Secret Recovery Phrase, it's gone forever. No one can help you get it back, no matter what they might say."
                )
            ),
            RevealQuizQuestion(
                id: 2,
                questionNumber: 2,
                totalQuestions: 2,
                prompt: String(
                    localized: "quiz_q2_prompt",
                    defaultValue: "If anyone, even a support agent, asks for your Secret Recovery Phrase..."
                ),
                options: [
                    String(localized: "quiz_q2_opt1", defaultValue: "You're being scammed"),
                    String(localized: "quiz_q2_opt2", defaultValue: "You should give it to them")
                ],
                correctOptionIndex: 0,
                correctHeading: String(
                    localized: "quiz_q2_correct_heading",
                    defaultValue: "Right! Never share your Secret Recovery Phrase with anyone"
                ),
                correctExplanation: String(
                    localized: "quiz_q2_correct_exp",
                    defaultValue: "Stable Channels support will never ask for your recovery phrase. Anyone asking for it is trying to steal your funds."
                ),
                incorrectHeading: String(
                    localized: "quiz_q2_incorrect_heading",
                    defaultValue: "Wrong! Never share your Secret Recovery Phrase with anyone"
                ),
                incorrectExplanation: String(
                    localized: "quiz_q2_incorrect_exp",
                    defaultValue: "No legitimate agent, developer, or service will ever ask for your recovery phrase. If they do, it is always a scam."
                )
            )
        ]
    }
}
