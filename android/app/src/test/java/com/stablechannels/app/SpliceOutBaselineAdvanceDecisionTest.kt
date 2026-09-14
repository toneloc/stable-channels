package com.stablechannels.app

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Pins AppState.advanceOnchainBaselineForCompletedSpliceOut() (#316 review follow-up): a
 * splice-out that pays one of our own addresses (a self-send) raises the on-chain balance by its
 * own settled amount. The deposit detector's frozen baseline must absorb exactly that amount, not
 * jump all the way to the post-refresh balance — otherwise a genuinely separate deposit landing
 * in the same window would be silently swallowed into the baseline and never detected, which is
 * the exact #316 bug this whole fix exists to prevent.
 *
 * AppState can't be unit-instantiated (it requires a live node + Android Application context), so
 * this tests the extracted pure function directly, the same one completeConfirmedSplice() calls.
 */
class SpliceOutBaselineAdvanceDecisionTest {

    @Test
    fun exactSelfSendSettlementCatchesBaselineUpToCurrent() {
        // Splice-out for 50,000 sats, self-send, no fee drift: balance rose by exactly that much.
        val result = AppState.advanceOnchainBaselineForCompletedSpliceOut(
            prevOnchainSats = 0L, currentSats = 50_000L, spliceAmountSats = 50_000L
        )
        assertEquals(50_000L, result)
    }

    @Test
    fun feeDriftNeverPushesBaselineAboveCurrentBalance() {
        // Actual on-chain rise (49_950) came in slightly below the splice's own amount (fees) —
        // the baseline must never exceed the real current balance.
        val result = AppState.advanceOnchainBaselineForCompletedSpliceOut(
            prevOnchainSats = 0L, currentSats = 49_950L, spliceAmountSats = 50_000L
        )
        assertEquals(49_950L, result)
    }

    @Test
    fun concurrentUnrelatedDepositRemainsVisibleAboveTheNewBaseline() {
        // A genuinely separate 10,000-sat deposit landed in the same window as a 50,000-sat
        // self-send splice-out: the new baseline must absorb only the splice's own 50,000, so the
        // remaining 10,000 is still detectable as a new deposit on the next tick.
        val result = AppState.advanceOnchainBaselineForCompletedSpliceOut(
            prevOnchainSats = 0L, currentSats = 60_000L, spliceAmountSats = 50_000L
        )
        assertEquals(50_000L, result)
    }

    @Test
    fun externalAddressSpliceOutLeavesBaselineAtCurrentBalance() {
        // A splice-out to an address that isn't ours doesn't raise our on-chain balance at all —
        // the coercion must fall back to the (unchanged) current balance, not overshoot it.
        val result = AppState.advanceOnchainBaselineForCompletedSpliceOut(
            prevOnchainSats = 20_000L, currentSats = 20_000L, spliceAmountSats = 50_000L
        )
        assertEquals(20_000L, result)
    }
}
