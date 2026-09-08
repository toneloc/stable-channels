package com.stablechannels.app

import com.stablechannels.app.services.TradeFailure
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Pins Android's local-refusal copy to the exact strings shipped by iOS
 * (TradeValidationError, PR #275) and desktop (LocalTradeAllocationError, PR #275).
 * The cross-platform mislabel fix (#272, PRs #274/#275) was reviewed as
 * "word-for-word consistent across all three clients" — this test turns that
 * point-in-time review claim into a standing contract on the Android side.
 * If a message must change, change it on all three platforms in the same PR.
 */
class TradeFailureCopyParityTest {

    @Test
    fun refusalCopyMatchesTheCrossPlatformContract() {
        assertEquals(
            "This channel is not ready to trade yet.",
            TradeFailure.INVALID_CHANNEL.userMessage()
        )
        assertEquals(
            "Enter a valid amount and try again.",
            TradeFailure.INVALID_AMOUNT.userMessage()
        )
        assertEquals(
            "The trade fee could not be calculated. Refresh the price and try again.",
            TradeFailure.FEE_UNAVAILABLE.userMessage()
        )
        assertEquals(
            "Your balance cannot cover this trade and its fee. Reduce the amount.",
            TradeFailure.FEE_EXCEEDS_BALANCE.userMessage()
        )
        assertEquals(
            "This trade cannot preserve the current channel allocation safely. " +
                "Settle the stability adjustment and retry.",
            TradeFailure.ALLOCATION_UNAVAILABLE.userMessage()
        )
    }

    @Test
    fun everyReasonIsPinned() {
        // If a new TradeFailure variant is added, this fails until its copy is
        // pinned above — and mirrored on iOS and desktop.
        assertEquals(5, TradeFailure.entries.size)
    }
}
