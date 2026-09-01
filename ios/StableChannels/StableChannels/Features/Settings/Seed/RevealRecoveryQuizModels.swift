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

/// Model representing a single self-custody educational guideline in the Learn More sheet.
struct SelfCustodyGuidelineItem: Identifiable, Equatable {
    let id: String
    let icon: String
    let title: String
    let description: String
}

/// Protocol providing guidelines for self-custody education.
protocol SelfCustodyGuidelinesProviderProtocol {
    func getGuidelines() -> [SelfCustodyGuidelineItem]
}

/// Default provider for self-custody guidelines.
struct DefaultSelfCustodyGuidelinesProvider: SelfCustodyGuidelinesProviderProtocol {
    func getGuidelines() -> [SelfCustodyGuidelineItem] {
        [
            SelfCustodyGuidelineItem(
                id: "master_key",
                icon: "key.horizontal.fill",
                title: String(localized: "guideline_master_key_title", defaultValue: "Your Words Are Your Master Key"),
                description: String(
                    localized: "guideline_master_key_desc",
                    defaultValue: "The 12 words mathematically derive all private keys in your wallet. Anyone with these words has full, irreversible control of your funds."
                )
            ),
            SelfCustodyGuidelineItem(
                id: "no_server_backups",
                icon: "icloud.slash",
                title: String(localized: "guideline_no_backups_title", defaultValue: "No Server Backups"),
                description: String(
                    localized: "guideline_no_backups_desc",
                    defaultValue: "Stable Channels is non-custodial. We never store or transmit your keys. If you lose your phrase, no one can recover your wallet."
                )
            ),
            SelfCustodyGuidelineItem(
                id: "beware_impersonators",
                icon: "exclamationmark.shield.fill",
                title: String(localized: "guideline_impersonators_title", defaultValue: "Beware of Impersonators"),
                description: String(
                    localized: "guideline_impersonators_desc",
                    defaultValue: "Support staff, developers, or bots will never ask for your recovery phrase. Never share it with anyone under any circumstance."
                )
            ),
            SelfCustodyGuidelineItem(
                id: "store_offline",
                icon: "lock.shield.fill",
                title: String(
                    localized: "guideline_store_offline_title",
                    defaultValue: "Store Offline on Physical Media"
                ),
                description: String(
                    localized: "guideline_store_offline_desc",
                    defaultValue: "Write your words on paper or stamp them on metal kept in a private, secure location. Never take screenshots or save digital copies in cloud notes."
                )
            )
        ]
    }
}
