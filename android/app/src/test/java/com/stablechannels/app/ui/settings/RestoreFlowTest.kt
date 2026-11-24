package com.stablechannels.app.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private val PHRASE = "abandon ".repeat(11) + "about"

class RestoreFlowTest {

    private fun checking(): Pair<RestoreFlow, RestoreCheck> {
        val flow = RestoreFlow()
        flow.open()
        flow.phrase = PHRASE
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
    fun `a blank phrase never starts a restore`() {
        val flow = RestoreFlow()
        assertFalse(flow.startApply(""))
        assertFalse(flow.startApply("   "))
        assertFalse(flow.isBusy)
    }

    @Test
    fun `only 12 or 24 words start the check`() {
        val flow = RestoreFlow()
        flow.phrase = "abandon about"
        assertNull(flow.startCheck())
        assertEquals("Seed phrase must be 12 or 24 words", flow.error)
        assertFalse(flow.isBusy)
    }

    /** Check A, leave the app, come back with the same phrase and start check B. */
    private fun restartedWithSamePhrase(): Triple<RestoreFlow, RestoreCheck, RestoreCheck> {
        val (flow, checkA) = checking()
        flow.onBackground()
        flow.open()
        flow.phrase = PHRASE
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
}
