package com.stablechannels.app

import com.stablechannels.app.services.TradeOutcome
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TradeOutcomeResolutionTest {
    @Test
    fun delayedTradeBecomesConfirmedOnlyAfterDurableAcceptance() {
        assertNull(TradeOutcome.fromStored("pending", null))
        assertNull(TradeOutcome.fromStored("uncertain", null))
        assertNull(TradeOutcome.fromStored("fee_paid", null))
        val outcome = TradeOutcome.fromStored("accepted", null)
        assertNotNull(outcome)
        assertTrue(outcome!!.accepted)
    }

    @Test
    fun delayedTradeBecomesRejectedOnlyAfterDurableRejection() {
        assertNull(TradeOutcome.fromStored("uncertain", null))
        val outcome = TradeOutcome.fromStored("rejected", "internal_failure")
        assertNotNull(outcome)
        assertFalse(outcome!!.accepted)
        assertFalse(outcome.sendFailed)
    }

    @Test
    fun definitiveFeeFailureAlsoResolvesTheResult() {
        val outcome = TradeOutcome.fromStored("send_failed", "internal_failure")
        assertNotNull(outcome)
        assertFalse(outcome!!.accepted)
        assertTrue(outcome.sendFailed)
    }
}
