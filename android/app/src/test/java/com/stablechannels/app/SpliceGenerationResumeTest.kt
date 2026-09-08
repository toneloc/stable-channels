package com.stablechannels.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins the exact regression all three reviewers found in the splice-generation guard: resuming a
 * pending splice confirmation must not bump [AppState]'s generation counter when this process is
 * already actively monitoring that same txid (foreground grace-period reconnect, or startup
 * racing a replayed SpliceNegotiated). Bumping in that case strands the counter one step ahead of
 * the value the still-running monitor captured — since the monitor is never re-armed for a
 * same-txid/active-job resume, nothing ever holds the new generation, so completeConfirmedSplice's
 * generation check permanently fails and the isSweeping lock wedges once that monitor confirms.
 *
 * AppState can't be unit-instantiated (it requires a live node + Android Application context), so
 * this tests the extracted pure decision function directly — the same one AppState.
 * resumePendingSpliceConfirmation() calls to decide whether to bump.
 */
class SpliceGenerationResumeTest {

    @Test
    fun resumingSameTxidWhileMonitorActiveSkipsBump() {
        val skip = AppState.shouldSkipGenerationBumpOnResume(
            monitorActive = true,
            monitoredTxid = "abc123",
            resumedTxid = "abc123"
        )
        assertTrue(skip)
    }

    @Test
    fun resumingSameTxidWithWhitespaceStillSkipsBump() {
        val skip = AppState.shouldSkipGenerationBumpOnResume(
            monitorActive = true,
            monitoredTxid = "abc123",
            resumedTxid = "  abc123  "
        )
        assertTrue(skip)
    }

    @Test
    fun resumingDifferentTxidWhileMonitorActiveDoesNotSkipBump() {
        // A genuinely different pending splice — this must still be treated as a new operation.
        val skip = AppState.shouldSkipGenerationBumpOnResume(
            monitorActive = true,
            monitoredTxid = "old-txid",
            resumedTxid = "new-txid"
        )
        assertFalse(skip)
    }

    @Test
    fun resumingWhileNoMonitorActiveDoesNotSkipBump() {
        // Full restart case: no live monitor survived, so this genuinely establishes a fresh one
        // and must capture a fresh generation.
        val skip = AppState.shouldSkipGenerationBumpOnResume(
            monitorActive = false,
            monitoredTxid = null,
            resumedTxid = "abc123"
        )
        assertFalse(skip)
    }

    @Test
    fun resumingWithNoMonitoredTxidDoesNotSkipBump() {
        val skip = AppState.shouldSkipGenerationBumpOnResume(
            monitorActive = true,
            monitoredTxid = null,
            resumedTxid = "abc123"
        )
        assertFalse(skip)
    }

    @Test
    fun resumingWithNullResumedTxidDoesNotSkipBump() {
        val skip = AppState.shouldSkipGenerationBumpOnResume(
            monitorActive = true,
            monitoredTxid = "abc123",
            resumedTxid = null
        )
        assertFalse(skip)
    }
}
