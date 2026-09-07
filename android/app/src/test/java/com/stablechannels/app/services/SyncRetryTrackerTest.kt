package com.stablechannels.app.services

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * NodeService's LDK event loop is strictly sequential: it won't fetch the next event until the
 * current one is acknowledged. A signed trade-sync message that can never commit (e.g. a stale
 * or not-yet-visible channel row) must not retry forever, or it silently blocks every event
 * after it, including Event.ChannelClosed. SyncRetryTracker bounds that by wall-clock time.
 */
class SyncRetryTrackerTest {

    @Test
    fun `keeps retrying before the max duration elapses`() {
        var now = 0L
        val tracker = SyncRetryTracker(maxDurationMs = 1_000L, nowMs = { now })

        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
        now = 500L
        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
    }

    @Test
    fun `gives up once the max duration has elapsed since the first attempt`() {
        var now = 0L
        val tracker = SyncRetryTracker(maxDurationMs = 1_000L, nowMs = { now })

        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
        now = 1_500L
        assertTrue(tracker.recordAttemptAndShouldGiveUp("hash1"))
    }

    @Test
    fun `tracks each key independently`() {
        var now = 0L
        val tracker = SyncRetryTracker(maxDurationMs = 1_000L, nowMs = { now })

        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
        now = 800L
        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash2"))
        now = 1_500L
        // hash1 started at 0, so 1500ms later it's past the bound.
        assertTrue(tracker.recordAttemptAndShouldGiveUp("hash1"))
        // hash2 started at 800, so only 700ms have elapsed — still within the bound.
        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash2"))
    }

    @Test
    fun `clear resets tracking so a later attempt starts a fresh window`() {
        var now = 0L
        val tracker = SyncRetryTracker(maxDurationMs = 1_000L, nowMs = { now })

        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
        tracker.clear("hash1")
        now = 1_500L
        // Without the clear this would give up immediately; with it, this is a fresh first attempt.
        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
    }

    @Test
    fun `giving up clears the key so a later message starts a fresh window`() {
        var now = 0L
        val tracker = SyncRetryTracker(maxDurationMs = 1_000L, nowMs = { now })

        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
        now = 1_500L
        assertTrue(tracker.recordAttemptAndShouldGiveUp("hash1"))
        // Same key attempted again right away should not immediately give up — it's a fresh window.
        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
    }
}
