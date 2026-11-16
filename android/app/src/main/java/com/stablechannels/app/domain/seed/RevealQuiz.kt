package com.stablechannels.app.domain.seed

/** One question of the quiz asked before the recovery phrase is shown. */
data class RevealQuizQuestion(
    val prompt: String,
    val options: List<String>,
    val correctIndex: Int,
    val correctHeading: String,
    val correctExplanation: String,
    val incorrectHeading: String,
    val incorrectExplanation: String,
) {
    fun isCorrect(index: Int): Boolean = index == correctIndex

    companion object {
        /** The questions asked, in order. Copy matches iOS (keys quiz_q1_*, quiz_q2_*). */
        val all: List<RevealQuizQuestion> =
            listOf(
                RevealQuizQuestion(
                    prompt = "If you lose your Secret Recovery Phrase, Stable Channels...",
                    options = listOf("Can get it back for you", "Can't help you"),
                    correctIndex = 1,
                    correctHeading = "Right! No one can help get your Secret Recovery Phrase back",
                    correctExplanation =
                        "Write it down, engrave it on metal, or keep it in multiple secret spots " +
                            "so you never lose it. If you lose it, it's gone forever.",
                    incorrectHeading =
                        "Wrong! No one can help get your Secret Recovery Phrase back",
                    incorrectExplanation =
                        "If you lose your Secret Recovery Phrase, it's gone forever. No one can " +
                            "help you get it back, no matter what they might say.",
                ),
                RevealQuizQuestion(
                    prompt =
                        "If anyone, even a support agent, asks for your Secret Recovery Phrase...",
                    options = listOf("You're being scammed", "You should give it to them"),
                    correctIndex = 0,
                    correctHeading = "Right! Never share your Secret Recovery Phrase with anyone",
                    correctExplanation =
                        "Stable Channels support will never ask for your recovery phrase. Anyone " +
                            "asking for it is trying to steal your funds.",
                    incorrectHeading = "Wrong! Never share your Secret Recovery Phrase with anyone",
                    incorrectExplanation =
                        "No legitimate agent, developer, or service will ever ask for your " +
                            "recovery phrase. If they do, it is always a scam.",
                ),
            )
    }
}

/** Where the reveal quiz flow is. */
sealed interface RevealQuizStep {
    data object Intro : RevealQuizStep

    data class Question(val index: Int) : RevealQuizStep

    data class Feedback(val questionIndex: Int, val isCorrect: Boolean) : RevealQuizStep

    data object Revealed : RevealQuizStep
}

/** Which way the last step change went, so the UI can pick its transition. */
enum class QuizNavigationDirection {
    FORWARD,
    BACKWARD,
}

/** One self-custody guideline shown in the Learn More sheet. */
data class SelfCustodyGuidelineItem(val id: String, val title: String, val description: String) {
    companion object {
        /** The guidelines, in order. Copy matches iOS, without the cloud backup sentence. */
        val all: List<SelfCustodyGuidelineItem> =
            listOf(
                SelfCustodyGuidelineItem(
                    id = "master_key",
                    title = "Your Words Are Your Master Key",
                    description =
                        "Your words mathematically derive all private keys in your wallet. " +
                            "Anyone with these words has full, irreversible control of your funds.",
                ),
                SelfCustodyGuidelineItem(
                    id = "never_stored",
                    title = "We never have it",
                    description =
                        "Stable Channels never stores your Secret Recovery Phrase and will never " +
                            "ask for it. Without your written copy, no one can recover your wallet.",
                ),
                SelfCustodyGuidelineItem(
                    id = "beware_impersonators",
                    title = "Beware of Impersonators",
                    description =
                        "Support staff, developers, or bots will never ask for your recovery " +
                            "phrase. Never share it with anyone under any circumstance.",
                ),
                SelfCustodyGuidelineItem(
                    id = "store_offline",
                    title = "Store Offline on Physical Media",
                    description =
                        "Write your words on paper or stamp them on metal kept in a private, " +
                            "secure location. Never take screenshots or save digital copies in " +
                            "cloud notes.",
                ),
            )
    }
}

/**
 * Immutable state of the reveal quiz. Each action returns the next state; actions that do not apply
 * to the current step return the state unchanged (apart from direction on [goBack]).
 */
@ConsistentCopyVisibility
data class RevealRecoveryQuiz
// Private so no caller can build or copy a state past the quiz; only the actions move it.
private constructor(val step: RevealQuizStep, val direction: QuizNavigationDirection) {
    constructor() : this(RevealQuizStep.Intro, QuizNavigationDirection.FORWARD)

    val canGoBack: Boolean
        get() = step is RevealQuizStep.Question || step is RevealQuizStep.Feedback

    /** "Question N of M" for the question at [index], counted from the question list. */
    fun progressText(index: Int): String = "Question ${index + 1} of ${QUESTIONS.size}"

    /** Reads the phrase only once the quiz is passed, so it is not held during the quiz. */
    fun mnemonic(provider: () -> String?): String? =
        if (step == RevealQuizStep.Revealed) provider()?.takeIf { it.isNotEmpty() } else null

    fun startQuiz(): RevealRecoveryQuiz = forwardTo(RevealQuizStep.Question(FIRST_QUESTION))

    /** Answers the question on screen; ignored outside a question step. */
    fun selectOption(index: Int): RevealRecoveryQuiz {
        val current = step as? RevealQuizStep.Question ?: return this
        val question = QUESTIONS.getOrNull(current.index) ?: return this
        return forwardTo(RevealQuizStep.Feedback(current.index, question.isCorrect(index)))
    }

    /** Next question or reveal when right, the same question again when wrong. */
    fun continueFromFeedback(): RevealRecoveryQuiz {
        val current = step as? RevealQuizStep.Feedback ?: return this
        if (!current.isCorrect) return forwardTo(RevealQuizStep.Question(current.questionIndex))
        val next = current.questionIndex + 1
        return forwardTo(
            if (next in QUESTIONS.indices) RevealQuizStep.Question(next)
            else RevealQuizStep.Revealed
        )
    }

    fun goBack(): RevealRecoveryQuiz {
        val previous =
            when (val current = step) {
                RevealQuizStep.Intro,
                RevealQuizStep.Revealed -> current
                is RevealQuizStep.Question ->
                    if (current.index > FIRST_QUESTION) RevealQuizStep.Question(current.index - 1)
                    else RevealQuizStep.Intro
                is RevealQuizStep.Feedback -> RevealQuizStep.Question(current.questionIndex)
            }
        return copy(step = previous, direction = QuizNavigationDirection.BACKWARD)
    }

    private fun forwardTo(next: RevealQuizStep) =
        copy(step = next, direction = QuizNavigationDirection.FORWARD)

    private companion object {
        const val FIRST_QUESTION = 0
        val QUESTIONS = RevealQuizQuestion.all
    }
}
