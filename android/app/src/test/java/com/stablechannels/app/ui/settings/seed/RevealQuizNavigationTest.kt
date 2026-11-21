package com.stablechannels.app.ui.settings.seed

import androidx.activity.ComponentActivity
import androidx.activity.ComponentDialog
import androidx.compose.animation.AnimatedContentTransitionScope.SlideDirection
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import com.stablechannels.app.domain.seed.QuizNavigationDirection
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowDialog

private const val MNEMONIC = "abandon ability able about above absent"

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class RevealQuizNavigationTest {

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    private var reads = 0
    private var closes = 0

    private fun showScreen() {
        compose.setContent {
            RevealRecoveryPhraseScreen(
                mnemonicProvider = {
                    reads++
                    MNEMONIC
                },
                onClose = { closes++ },
            )
        }
    }

    private fun tap(text: String) {
        compose.onNodeWithText(text).performClick()
        compose.waitForIdle()
    }

    @Test
    fun `the phrase is read only once the quiz is passed`() {
        showScreen()
        compose.onNodeWithText("Get started").assertExists()
        assertEquals(0, reads)

        tap("Get started")
        compose.onNodeWithText("Question 1 of 2").assertExists()
        assertEquals(0, reads)

        tap("Can't help you")
        compose.onNodeWithText("Correct!").assertExists()
        assertEquals(0, reads)

        tap("Continue")
        compose.onNodeWithText("Question 2 of 2").assertExists()
        assertEquals(0, reads)

        tap("You're being scammed")
        tap("Continue")
        tap("Tap to reveal")
        compose.onNodeWithText("abandon").assertExists()
        assertEquals(1, reads)
    }

    @Test
    fun `a wrong answer then try again returns to the same question`() {
        showScreen()
        tap("Get started")

        tap("Can get it back for you")
        compose.onNodeWithText("Incorrect!").assertExists()
        tap("Try again")

        compose.onNodeWithText("Question 1 of 2").assertExists()
        compose.onNodeWithText("Can get it back for you").assertExists()
    }

    @Test
    fun `system back on the intro closes`() {
        showScreen()

        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

        assertEquals(1, closes)
    }

    @Test
    fun `the chevron on the intro closes`() {
        showScreen()

        compose.onNodeWithContentDescription("Close").performClick()

        assertEquals(1, closes)
    }

    @Test
    fun `forward steps enter from the trailing edge and back steps from the leading edge`() {
        assertEquals(SlideDirection.Start, quizSlideDirection(QuizNavigationDirection.FORWARD))
        assertEquals(SlideDirection.End, quizSlideDirection(QuizNavigationDirection.BACKWARD))
    }

    @Test
    fun `a tap on the question that is leaving does not answer the one shown`() {
        showScreen()
        tap("Get started")
        tap("Can't help you")
        tap("Continue")

        compose.mainClock.autoAdvance = false
        compose.onNodeWithContentDescription("Back").performClick()
        compose.mainClock.advanceTimeBy(100)
        // An option of the second question, still fading out over the first.
        compose.onNodeWithText("You're being scammed").performClick()
        compose.mainClock.autoAdvance = true
        compose.waitForIdle()

        compose.onNodeWithText("Question 1 of 2").assertExists()
        compose.onNodeWithText("Incorrect!").assertDoesNotExist()
    }

    @Test
    fun `back inside the dialog steps back instead of closing it`() {
        compose.setContent {
            RevealSecretPhraseDialog(mnemonicProvider = { MNEMONIC }, onDismiss = { closes++ })
        }
        tap("Get started")

        compose.runOnUiThread {
            (ShadowDialog.getLatestDialog() as ComponentDialog)
                .onBackPressedDispatcher
                .onBackPressed()
        }
        compose.waitForIdle()

        compose.onNodeWithText("Get started").assertExists()
        assertEquals(0, closes)
    }

    @Test
    fun `at a large text size the answers scroll with the question instead of being cut off`() {
        compose.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(
                LocalDensity provides Density(density.density, fontScale = 2f)
            ) {
                RevealRecoveryPhraseScreen(mnemonicProvider = { MNEMONIC }, onClose = {})
            }
        }
        tap("Get started")

        compose.onNodeWithText("Can't help you").performScrollTo().performClick()
        compose.waitForIdle()

        compose.onNodeWithText("Correct!").assertExists()
    }
}
