package com.stablechannels.app.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private val PHRASE = "abandon ".repeat(11) + "about"

class RestoreFlowTest {

    private fun checking(): Pair<RestoreFlow, String> {
        val flow = RestoreFlow()
        flow.open()
        flow.onPhraseChanged(PHRASE)
        return flow to flow.startCheck()!!
    }

    @Test
    fun `leaving the app during the channel check clears the phrase and opens no warning`() {
        val (flow, input) = checking()
        flow.onBackground()
        assertEquals("", flow.phrase)
        assertFalse(flow.isOpen)
        assertFalse(flow.isBusy)

        assertNull(flow.onChecked(input, channelExists = true))
        assertNull(flow.warning)
        assertNull(flow.onChecked(input, channelExists = false))
    }

    @Test
    fun `a check that ends in the foreground warns or restores`() {
        val (flow, input) = checking()
        assertNull(flow.onChecked(input, channelExists = true))
        assertEquals(RestoreWarning.OPEN_CHANNEL, flow.warning)
        assertEquals(PHRASE, flow.confirmWarning())

        val (other, otherInput) = checking()
        assertEquals(otherInput, other.onChecked(otherInput, channelExists = false))
        assertNull(other.onChecked(otherInput, channelExists = false))
    }

    @Test
    fun `leaving the app closes an open warning and its phrase`() {
        val (flow, input) = checking()
        flow.onChecked(input, channelExists = null)
        assertEquals(RestoreWarning.UNVERIFIED, flow.warning)
        flow.onBackground()
        assertNull(flow.warning)
        assertNull(flow.confirmWarning())
    }

    @Test
    fun `a restore already applying keeps its dialog but drops the phrase`() {
        val (flow, input) = checking()
        assertTrue(flow.startApply(input))
        flow.onBackground()
        assertEquals("", flow.phrase)
        assertTrue(flow.isOpen)
        assertTrue(flow.isBusy)
        assertFalse(flow.startApply(input))
    }

    @Test
    fun `a blank phrase never starts a restore`() {
        val flow = RestoreFlow()
        assertFalse(flow.startApply(""))
        assertFalse(flow.startApply("   "))
        assertFalse(flow.isBusy)
    }

    @Test
    fun `only 12 or 24 words start the check`() {
        val flow = RestoreFlow()
        flow.onPhraseChanged("abandon about")
        assertNull(flow.startCheck())
        assertEquals("Seed phrase must be 12 or 24 words", flow.error)
        assertFalse(flow.isBusy)
    }

    @Test
    fun `typing the phrase fills the word cells and locks them`() {
        val flow = RestoreFlow()
        flow.onPhraseChanged(" Abandon  about ")
        assertEquals(listOf("abandon", "about") + List(22) { "" }, flow.wordFields)
        assertTrue(flow.isWordFieldsReadOnly)
    }

    @Test
    fun `editing a cell rebuilds the phrase and keeps the cells editable`() {
        val flow = RestoreFlow()
        flow.onWordChanged(0, "abandon")
        flow.onWordChanged(2, " about\n")
        assertEquals("abandon about", flow.phrase)
        assertEquals(listOf("abandon", "", "about"), flow.wordFields.take(3))
        assertFalse(flow.isWordFieldsReadOnly)
    }

    @Test
    fun `a phrase pasted into one cell spreads across the cells from there`() {
        val flow = RestoreFlow()
        flow.onWordChanged(0, "zoo")
        flow.onWordChanged(1, " Abandon ability\nable ")
        assertEquals(listOf("zoo", "abandon", "ability", "able", ""), flow.wordFields.take(5))
        assertEquals("zoo abandon ability able", flow.phrase)

        flow.onWordChanged(0, PHRASE)
        assertEquals(PHRASE, flow.phrase)
        assertEquals(12, flow.wordFields.count { it.isNotEmpty() })
    }

    @Test
    fun `clearing the phrase unlocks the word cells`() {
        val flow = RestoreFlow()
        flow.onPhraseChanged(PHRASE)
        flow.onPhraseChanged("")
        assertFalse(flow.isWordFieldsReadOnly)
    }

    @Test
    fun `a numbered paste shows the format error`() {
        val flow = RestoreFlow()
        assertNull(flow.formatError)
        flow.onPhraseChanged(
            PHRASE.split(" ").mapIndexed { i, w -> "${i + 1}. $w" }.joinToString(" ")
        )
        assertFalse(flow.canRestore)
        assertEquals("Enter only the words, without numbers or punctuation", flow.formatError)
        flow.onPhraseChanged(PHRASE)
        assertNull(flow.formatError)
    }

    @Test
    fun `restore is allowed only with 12 or 24 letter-only words and nothing running`() {
        val flow = RestoreFlow()
        flow.onPhraseChanged("abandon about")
        assertFalse(flow.canRestore)
        flow.onPhraseChanged(PHRASE)
        assertTrue(flow.canRestore)
        flow.onPhraseChanged(PHRASE.replace("about", "ab0ut"))
        assertFalse(flow.canRestore)
        flow.onPhraseChanged(PHRASE)
        flow.startCheck()
        assertFalse(flow.canRestore)
    }

    @Test
    fun `the check uses the normalized phrase and still matches it`() {
        val flow = RestoreFlow()
        flow.onPhraseChanged("  " + PHRASE.uppercase().replace(" ", "\n "))
        val input = flow.startCheck()
        assertEquals(PHRASE, input)
        assertEquals(PHRASE, flow.onChecked(input!!, channelExists = false))
    }

    @Test
    fun `leaving the app or closing clears the word cells`() {
        val flow = RestoreFlow()
        flow.open()
        flow.onPhraseChanged(PHRASE)
        flow.onBackground()
        assertTrue(flow.wordFields.all { it.isEmpty() })
        assertFalse(flow.isWordFieldsReadOnly)

        flow.open()
        flow.onWordChanged(0, "abandon")
        flow.close()
        assertTrue(flow.wordFields.all { it.isEmpty() })
        assertEquals("", flow.phrase)
    }
}
