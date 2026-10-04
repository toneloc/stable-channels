package com.stablechannels.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins AppState.shouldClearLastCloseTxid() (#316 follow-up): lastCloseTxid must only clear once
 * THIS close's own funds have reached the required confirmation count — not merely once the
 * wallet's aggregate spendable balance is positive again, which can already be true from other,
 * unrelated funds while the close's own output is still confirming (that coarser check would
 * mislabel a later, unrelated on-chain receive as still being the old close).
 *
 * AppState can't be unit-instantiated (it requires a live node + Android Application context), so
 * this tests the extracted pure decision function directly, the same one refreshBalances() calls.
 */
class CloseTxidClearDecisionTest {

    @Test
    fun nullConfirmationsDoesNotClear() {
        // The close's row can't be found yet (e.g. txid not resolved) — must not clear.
        assertFalse(AppState.shouldClearLastCloseTxid(confirmations = null, required = 6))
    }

    @Test
    fun confirmationsBelowRequiredDoesNotClear() {
        assertFalse(AppState.shouldClearLastCloseTxid(confirmations = 5, required = 6))
    }

    @Test
    fun confirmationsAtRequiredClears() {
        assertTrue(AppState.shouldClearLastCloseTxid(confirmations = 6, required = 6))
    }

    @Test
    fun confirmationsAboveRequiredClears() {
        assertTrue(AppState.shouldClearLastCloseTxid(confirmations = 7, required = 6))
    }

    @Test
    fun zeroConfirmationsDoesNotClear() {
        // Guards specifically against the earlier "aggregate spendable > 0" bug: a fresh close
        // row genuinely at 0 confirmations must never be treated as cleared.
        assertFalse(AppState.shouldClearLastCloseTxid(confirmations = 0, required = 6))
    }
}
