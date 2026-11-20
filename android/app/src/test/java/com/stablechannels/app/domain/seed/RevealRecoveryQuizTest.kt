package com.stablechannels.app.domain.seed

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RevealRecoveryQuizTest {

    private val testMnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"

    private fun revealedQuiz(): RevealRecoveryQuiz =
        RevealRecoveryQuiz()
            .startQuiz()
            .selectOption(1)
            .continueFromFeedback()
            .selectOption(0)
            .continueFromFeedback()

    @Test
    fun `initial state is intro`() {
        val quiz = RevealRecoveryQuiz()
        assertEquals(RevealQuizStep.Intro, quiz.step)
        assertFalse(quiz.canGoBack)
        assertEquals("Question 1 of 2", quiz.progressText(0))
    }

    @Test
    fun `phrase is read only once revealed`() {
        var reads = 0
        val provider = {
            reads += 1
            testMnemonic
        }
        val started = RevealRecoveryQuiz().startQuiz()
        assertNull(started.mnemonic(provider))
        assertEquals(0, reads)

        val revealed =
            started.selectOption(1).continueFromFeedback().selectOption(0).continueFromFeedback()
        assertEquals(testMnemonic, revealed.mnemonic(provider))
        assertEquals(1, reads)
    }

    @Test
    fun `revealed without saved phrase has no phrase`() {
        val quiz = revealedQuiz()
        assertEquals(RevealQuizStep.Revealed, quiz.step)
        assertNull(quiz.mnemonic { "" })
        assertNull(quiz.mnemonic { null })
    }

    @Test
    fun `start quiz moves to first question`() {
        val quiz = RevealRecoveryQuiz().startQuiz()
        assertEquals(RevealQuizStep.Question(0), quiz.step)
        assertTrue(quiz.canGoBack)
    }

    @Test
    fun `answer question 1 correct transition`() {
        val answered = RevealRecoveryQuiz().startQuiz().selectOption(1)
        assertEquals(RevealQuizStep.Feedback(questionIndex = 0, isCorrect = true), answered.step)

        assertEquals(RevealQuizStep.Question(1), answered.continueFromFeedback().step)
    }

    @Test
    fun `answer question 1 incorrect transition and retry`() {
        val answered = RevealRecoveryQuiz().startQuiz().selectOption(0)
        assertEquals(RevealQuizStep.Feedback(questionIndex = 0, isCorrect = false), answered.step)

        assertEquals(RevealQuizStep.Question(0), answered.continueFromFeedback().step)
    }

    @Test
    fun `wrong answer on last question never reveals`() {
        val quiz =
            RevealRecoveryQuiz()
                .startQuiz()
                .selectOption(1)
                .continueFromFeedback()
                .selectOption(1)
                .continueFromFeedback()
        assertEquals(RevealQuizStep.Question(1), quiz.step)
    }

    @Test
    fun `full happy path reveals phrase`() {
        val second = RevealRecoveryQuiz().startQuiz().selectOption(1).continueFromFeedback()
        assertEquals(RevealQuizStep.Question(1), second.step)

        val answered = second.selectOption(0)
        assertEquals(RevealQuizStep.Feedback(questionIndex = 1, isCorrect = true), answered.step)

        val revealed = answered.continueFromFeedback()
        assertEquals(RevealQuizStep.Revealed, revealed.step)
        assertFalse(revealed.canGoBack)
    }

    @Test
    fun `select option outside question step is no-op`() {
        assertEquals(RevealQuizStep.Intro, RevealRecoveryQuiz().selectOption(1).step)

        val quiz = RevealRecoveryQuiz().startQuiz().selectOption(0).selectOption(1)
        assertEquals(RevealQuizStep.Feedback(questionIndex = 0, isCorrect = false), quiz.step)
    }

    @Test
    fun `continue outside feedback step is no-op`() {
        assertEquals(RevealQuizStep.Intro, RevealRecoveryQuiz().continueFromFeedback().step)

        val quiz = RevealRecoveryQuiz().startQuiz().continueFromFeedback()
        assertEquals(RevealQuizStep.Question(0), quiz.step)
    }

    @Test
    fun `go back from question 1 returns to intro`() {
        val quiz = RevealRecoveryQuiz().startQuiz().goBack()
        assertEquals(RevealQuizStep.Intro, quiz.step)
        assertEquals(QuizNavigationDirection.BACKWARD, quiz.direction)
        assertFalse(quiz.canGoBack)
    }

    @Test
    fun `go back from question 2 returns to question 1`() {
        val second = RevealRecoveryQuiz().startQuiz().selectOption(1).continueFromFeedback()
        assertEquals(RevealQuizStep.Question(1), second.step)

        assertEquals(RevealQuizStep.Question(0), second.goBack().step)
    }

    @Test
    fun `go back from feedback returns to question`() {
        val quiz = RevealRecoveryQuiz().startQuiz().selectOption(0).goBack()
        assertEquals(RevealQuizStep.Question(0), quiz.step)
    }

    @Test
    fun `forward actions reset direction after going back`() {
        val quiz = RevealRecoveryQuiz().startQuiz().goBack().startQuiz()
        assertEquals(QuizNavigationDirection.FORWARD, quiz.direction)
    }

    @Test
    fun `go back when revealed is no-op`() {
        val quiz = revealedQuiz()
        assertEquals(RevealQuizStep.Revealed, quiz.step)
        assertFalse(quiz.canGoBack)

        assertEquals(RevealQuizStep.Revealed, quiz.goBack().step)
    }

    @Test
    fun `progress text counts from the question list`() {
        val quiz = RevealRecoveryQuiz()
        assertEquals("Question 1 of 2", quiz.progressText(0))
        assertEquals("Question 2 of 2", quiz.progressText(1))
    }

    @Test
    fun `go back from intro is no-op`() {
        assertEquals(RevealQuizStep.Intro, RevealRecoveryQuiz().goBack().step)
    }

    @Test
    fun guidelines() {
        val guidelines = SelfCustodyGuidelineItem.all
        assertEquals(
            listOf("master_key", "never_stored", "beware_impersonators", "store_offline"),
            guidelines.map { it.id },
        )
        assertEquals(
            "Stable Channels never stores your Secret Recovery Phrase and will never ask for it. " +
                "Without your written copy, no one can recover your wallet.",
            guidelines[1].description,
        )
        assertFalse(guidelines.any { it.description.contains("iCloud") })
    }

    @Test
    fun `questions keep the iOS copy and correct answers`() {
        val (first, second) = RevealQuizQuestion.all
        assertEquals("If you lose your Secret Recovery Phrase, Stable Channels...", first.prompt)
        assertEquals(listOf("Can get it back for you", "Can't help you"), first.options)
        assertTrue(first.isCorrect(1))
        assertFalse(first.isCorrect(0))
        assertEquals(
            "Right! No one can help get your Secret Recovery Phrase back",
            first.correctHeading,
        )
        assertEquals(
            "If anyone, even a support agent, asks for your Secret Recovery Phrase...",
            second.prompt,
        )
        assertEquals(listOf("You're being scammed", "You should give it to them"), second.options)
        assertTrue(second.isCorrect(0))
        assertFalse(second.isCorrect(1))
        assertEquals(
            "Wrong! Never share your Secret Recovery Phrase with anyone",
            second.incorrectHeading,
        )
    }

    @Test
    fun `out of range option is a wrong answer`() {
        val started = RevealRecoveryQuiz().startQuiz()

        assertEquals(RevealQuizStep.Feedback(0, isCorrect = false), started.selectOption(2).step)
        assertEquals(RevealQuizStep.Feedback(0, isCorrect = false), started.selectOption(-1).step)
    }
}
