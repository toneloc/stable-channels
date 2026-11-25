package com.stablechannels.app.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private val PHRASE = "abandon ".repeat(11) + "about"

class RestoreFlowTest {

    private fun checking(): Pair<RestoreFlow, RestoreCheck> {
        val flow = RestoreFlow { true }
        flow.open()
        flow.onPhraseChanged(PHRASE)
        return flow to flow.startCheck()!!
    }

    @Test
    fun `leaving the app during the channel check clears the phrase and opens no warning`() {
        val (flow, check) = checking()
        flow.onBackground()
        assertEquals("", flow.phrase)
        assertFalse(flow.isOpen)
        assertFalse(flow.isBusy)

        assertNull(flow.onChecked(check, channelExists = true))
        assertNull(flow.warning)
        assertNull(flow.onChecked(check, channelExists = false))
    }

    @Test
    fun `a check that ends in the foreground warns or restores`() {
        val (flow, check) = checking()
        assertNull(flow.onChecked(check, channelExists = true))
        assertEquals(RestoreWarning.OPEN_CHANNEL, flow.warning)
        assertEquals(PHRASE, flow.confirmWarning())

        val (other, otherCheck) = checking()
        assertEquals(otherCheck.phrase, other.onChecked(otherCheck, channelExists = false))
        assertNull(other.onChecked(otherCheck, channelExists = false))
    }

    @Test
    fun `leaving the app closes an open warning and its phrase`() {
        val (flow, check) = checking()
        flow.onChecked(check, channelExists = null)
        assertEquals(RestoreWarning.UNVERIFIED, flow.warning)
        flow.onBackground()
        assertNull(flow.warning)
        assertNull(flow.confirmWarning())
    }

    @Test
    fun `a restore already applying keeps its dialog but drops the phrase`() {
        val (flow, check) = checking()
        assertTrue(flow.startApply(check.phrase))
        flow.onBackground()
        assertEquals("", flow.phrase)
        assertTrue(flow.isOpen)
        assertTrue(flow.isBusy)
        assertFalse(flow.startApply(check.phrase))
    }

    @Test
    fun `a restore that finishes after the app was left closes its dialog`() {
        val (flow, check) = checking()
        assertTrue(flow.startApply(check.phrase))
        flow.onBackground()
        flow.onApplied()
        assertFalse(flow.isOpen)
        assertFalse(flow.isBusy)
    }

    @Test
    fun `a blank phrase never starts a restore`() {
        val flow = RestoreFlow { true }
        assertFalse(flow.startApply(""))
        assertFalse(flow.startApply("   "))
        assertFalse(flow.isBusy)
    }

    @Test
    fun `only 12 or 24 words start the check`() {
        val flow = RestoreFlow { true }
        flow.onPhraseChanged("abandon about")
        assertNull(flow.startCheck())
        assertEquals("Seed phrase must be 12 or 24 words", flow.error)
        assertFalse(flow.isBusy)
    }

    @Test
    fun `a phrase that fails BIP39 stops at a clear error before the channel check`() {
        val flow = RestoreFlow { it != PHRASE }
        flow.onPhraseChanged(PHRASE)
        assertNull(flow.startCheck())
        assertEquals("Invalid seed words", flow.error)
        assertFalse(flow.isBusy)
        assertNull(flow.warning)
    }

    @Test
    fun `typing the phrase fills the word cells and locks them`() {
        val flow = RestoreFlow { true }
        flow.onPhraseChanged(" Abandon  about ")
        assertEquals(listOf("abandon", "about") + List(22) { "" }, flow.wordFields)
        assertTrue(flow.isWordFieldsReadOnly)
    }

    @Test
    fun `editing a cell rebuilds the phrase and keeps the cells editable`() {
        val flow = RestoreFlow { true }
        flow.onWordChanged(0, "abandon")
        flow.onWordChanged(2, " about\n")
        assertEquals("abandon about", flow.phrase)
        assertEquals(listOf("abandon", "", "about"), flow.wordFields.take(3))
        assertFalse(flow.isWordFieldsReadOnly)
    }

    @Test
    fun `a phrase pasted into one cell spreads across the cells from there`() {
        val flow = RestoreFlow { true }
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
        val flow = RestoreFlow { true }
        flow.onPhraseChanged(PHRASE)
        flow.onPhraseChanged("")
        assertFalse(flow.isWordFieldsReadOnly)
    }

    @Test
    fun `a numbered paste shows the format error`() {
        val flow = RestoreFlow { true }
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
        val flow = RestoreFlow { true }
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
        val flow = RestoreFlow { true }
        flow.onPhraseChanged("  " + PHRASE.uppercase().replace(" ", "\n "))
        val check = flow.startCheck()!!
        assertEquals(PHRASE, check.phrase)
        assertEquals(PHRASE, flow.onChecked(check, channelExists = false))
    }

    @Test
    fun `leaving the app or closing clears the word cells`() {
        val flow = RestoreFlow { true }
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

    /** Check A, leave the app, come back with the same phrase and start check B. */
    private fun restartedWithSamePhrase(): Triple<RestoreFlow, RestoreCheck, RestoreCheck> {
        val (flow, checkA) = checking()
        flow.onBackground()
        flow.open()
        flow.onPhraseChanged(PHRASE)
        return Triple(flow, checkA, flow.startCheck()!!)
    }

    @Test
    fun `an old check answering no channel leaves the newer check for the same phrase alone`() {
        val (flow, checkA, checkB) = restartedWithSamePhrase()

        assertNull(flow.onChecked(checkA, channelExists = false))
        assertNull(flow.warning)
        assertTrue(flow.isBusy)

        assertNull(flow.onChecked(checkB, channelExists = true))
        assertEquals(RestoreWarning.OPEN_CHANNEL, flow.warning)
        assertFalse(flow.isBusy)
    }

    @Test
    fun `an old check answering open channel leaves the newer check for the same phrase alone`() {
        val (flow, checkA, checkB) = restartedWithSamePhrase()

        assertNull(flow.onChecked(checkA, channelExists = true))
        assertNull(flow.warning)
        assertTrue(flow.isBusy)

        assertEquals(PHRASE, flow.onChecked(checkB, channelExists = false))
        assertNull(flow.warning)
        assertFalse(flow.isBusy)
    }

    private val OTHER_PHRASE = "zoo ".repeat(11) + "wrong"

    @Test
    fun `restore anyway restores the phrase that was checked, not one edited after`() {
        val (flow, check) = checking()
        assertNull(flow.onChecked(check, channelExists = true))

        flow.onPhraseChanged(OTHER_PHRASE)

        assertEquals(PHRASE, flow.confirmWarning())
    }

    @Test
    fun `a second tap while a check runs starts nothing and keeps the first check`() {
        val (flow, check) = checking()

        assertNull(flow.startCheck())
        assertNull(flow.error)

        assertEquals(PHRASE, flow.onChecked(check, channelExists = false))
    }

    @Test
    fun `a check whose phrase was edited ends without restoring or warning`() {
        val (flow, check) = checking()
        flow.onPhraseChanged(OTHER_PHRASE)

        assertNull(flow.onChecked(check, channelExists = false))
        assertNull(flow.warning)
        assertFalse(flow.isBusy)
    }

    @Test
    fun `only a check that opens a warning reports it`() {
        val (flow, checkA, checkB) = restartedWithSamePhrase()
        val warned = mutableListOf<RestoreWarning>()

        assertNull(flow.onChecked(checkA, channelExists = true) { warned += it })
        assertEquals(emptyList<RestoreWarning>(), warned)

        assertNull(flow.onChecked(checkB, channelExists = true) { warned += it })
        assertEquals(listOf(RestoreWarning.OPEN_CHANNEL), warned)
    }

    @Test
    fun `a restore or an unverified check reports only the warning it opens`() {
        val warned = mutableListOf<RestoreWarning>()
        val (restoring, check) = checking()
        assertEquals(PHRASE, restoring.onChecked(check, channelExists = false) { warned += it })
        assertEquals(emptyList<RestoreWarning>(), warned)

        val (unverified, other) = checking()
        assertNull(unverified.onChecked(other, channelExists = null) { warned += it })
        assertEquals(listOf(RestoreWarning.UNVERIFIED), warned)
    }
}
