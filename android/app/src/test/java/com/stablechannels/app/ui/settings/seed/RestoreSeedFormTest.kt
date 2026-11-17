package com.stablechannels.app.ui.settings.seed

import android.text.InputType
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.stablechannels.app.ui.settings.RestoreFlow
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

private val TWELVE = List(12) { "abandon" }.joinToString(" ")

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class RestoreSeedFormTest {

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    private val restore = RestoreFlow { true }
    private var restores = 0

    private fun showForm() {
        compose.setContent { RestoreSeedForm(restore = restore, onRestore = { restores++ }) }
    }

    /** Focuses a field and returns the EditorInfo the keyboard would receive for it. */
    private fun editorInfoOf(field: SemanticsNodeInteraction): EditorInfo {
        field.performScrollTo().performClick()
        compose.waitForIdle()
        val content = compose.activity.findViewById<ViewGroup>(android.R.id.content)
        val composeView = (content.getChildAt(0) as ViewGroup).getChildAt(0)
        return EditorInfo().also { composeView.onCreateInputConnection(it) }
    }

    private fun assertSeedKeyboard(info: EditorInfo) {
        assertEquals(
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD,
            info.inputType and InputType.TYPE_MASK_VARIATION,
        )
        val learningFlags =
            InputType.TYPE_TEXT_FLAG_AUTO_CORRECT or
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES or
                InputType.TYPE_TEXT_FLAG_CAP_WORDS or
                InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS
        assertEquals(0, info.inputType and learningFlags)
        assertEquals(
            EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING,
            info.imeOptions and EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING,
        )
    }

    @Test
    fun `restore stays disabled until 12 or 24 words are entered`() {
        showForm()
        compose.onNodeWithText("Restore").assertIsNotEnabled()
        restore.onPhraseChanged("abandon about")
        compose.onNodeWithText("Restore").assertIsNotEnabled()

        restore.onPhraseChanged(TWELVE)
        compose.onNodeWithText("Restore").performScrollTo().assertIsEnabled().performClick()
        assertEquals(1, restores)
    }

    @Test
    fun `the grid shows 12 cells, then 24 once the phrase grows past 12 words`() {
        showForm()
        compose.onNodeWithText("12.").assertExists()
        compose.onNodeWithText("13.").assertDoesNotExist()
        restore.onPhraseChanged("$TWELVE about")
        compose.onNodeWithText("24.").assertExists()
    }

    @Test
    fun `a running restore shows its progress label`() {
        showForm()
        restore.onPhraseChanged(TWELVE)
        restore.startCheck()
        compose.onNodeWithText("Restoring...").assertIsNotEnabled()
    }

    @Test
    fun `the copy matches the iOS strings`() {
        showForm()
        compose.onNodeWithText("Restore Seed").assertExists()
        compose.onNodeWithText("Enter your 12 or 24 word seed phrase").assertExists()
        compose.onNodeWithText("word1 word2 word3...", useUnmergedTree = true).assertExists()
    }

    @Test
    fun `each word cell has an accessibility label`() {
        showForm()
        compose.onNodeWithContentDescription("Word 1").assertExists()
        compose.onNodeWithContentDescription("Word 12").assertExists()
        restore.onPhraseChanged(TWELVE)
        compose.onNodeWithContentDescription("Word 12").assertExists()
    }

    @Test
    fun `a numbered paste shows the format error`() {
        showForm()
        restore.onPhraseChanged("1. abandon 2. ability")
        compose
            .onNodeWithText("Enter only the words, without numbers or punctuation")
            .assertExists()
    }

    @Test
    fun `the phrase field and word cells use the no-learning seed keyboard`() {
        showForm()
        assertSeedKeyboard(editorInfoOf(compose.onAllNodes(hasSetTextAction()).onFirst()))
        assertSeedKeyboard(
            editorInfoOf(compose.onAllNodesWithContentDescription("Word 1").onFirst())
        )
    }
}
