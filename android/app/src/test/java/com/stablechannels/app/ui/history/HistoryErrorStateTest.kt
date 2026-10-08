package com.stablechannels.app.ui.history

import com.stablechannels.app.services.ConfirmationPollResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class HistoryErrorStateTest {
    private val tipError = "Couldn't reach the block explorer. Pull to try again."

    @Test
    fun `chain tip failure sets the banner`() {
        val errors = HistoryErrorState()
        errors.onConfirmationResult(ConfirmationPollResult.ChainTipUnavailable)
        assertEquals(listOf(tipError), errors.visibleErrors)
    }

    @Test
    fun `only a later clean pass clears the banner`() {
        val errors = HistoryErrorState()
        errors.onConfirmationResult(ConfirmationPollResult.ChainTipUnavailable)

        errors.onConfirmationResult(ConfirmationPollResult.TimedOut)
        assertEquals(1, errors.visibleErrors.size)
        errors.onConfirmationResult(ConfirmationPollResult.DatabaseUnavailable)
        assertEquals(1, errors.visibleErrors.size)

        errors.onConfirmationResult(ConfirmationPollResult.Completed(0))
        assertTrue(errors.visibleErrors.isEmpty())
    }

    @Test
    fun `database reload does not hide a failed confirmation check`() {
        val errors = HistoryErrorState()
        errors.onConfirmationResult(ConfirmationPollResult.ChainTipUnavailable)
        errors.onLoadFailed("load failed")
        errors.onLoadSucceeded()
        assertEquals(listOf(tipError), errors.visibleErrors)
    }

    @Test
    fun `clear and failed refresh`() {
        val errors = HistoryErrorState()
        errors.onRefreshFailed()
        errors.onLoadFailed("load failed")
        assertEquals(2, errors.visibleErrors.size)
        errors.clear()
        assertTrue(errors.visibleErrors.isEmpty())
    }
}
