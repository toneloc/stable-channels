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
        flow.phrase = PHRASE
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
        flow.phrase = "abandon about"
        assertNull(flow.startCheck())
        assertEquals("Seed phrase must be 12 or 24 words", flow.error)
        assertFalse(flow.isBusy)
    }
}
