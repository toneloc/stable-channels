package com.stablechannels.app.services

import org.junit.Assert.assertEquals
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

    // Regression: a real device saw this exact scenario — the same stuck payment_hash retried
    // for over 50 minutes across many app/process restarts with zero give-ups, because the
    // in-memory-only first-attempt clock reset every restart. LDK durably persists an un-acked
    // event and redelivers it after restart, so the window must survive a fresh tracker instance
    // backed by the same persisted store, not just survive within one process's lifetime.
    @Test
    fun `first-attempt time survives recreating the tracker against the same backing store`() {
        val store = mutableMapOf<String, Long>()
        var now = 0L
        fun newTracker() = SyncRetryTracker(
            maxDurationMs = 1_000L,
            nowMs = { now },
            loadFirstAttempt = { key -> store[key] },
            saveFirstAttempt = { key, ts -> store[key] = ts },
            clearFirstAttempt = { key -> store.remove(key) }
        )

        // First "process": records the first attempt and persists it.
        assertFalse(newTracker().recordAttemptAndShouldGiveUp("hash1"))
        assertEquals(0L, store["hash1"])

        // Process "restarts" (fresh tracker instance, same backing store). A naive in-memory-only
        // implementation would treat this as a brand new first attempt at time 500 and never give
        // up; this must instead recall that the window actually started at 0.
        now = 500L
        assertFalse(newTracker().recordAttemptAndShouldGiveUp("hash1"))

        now = 1_500L
        assertTrue(newTracker().recordAttemptAndShouldGiveUp("hash1"))
        assertFalse(store.containsKey("hash1"))
    }

    @Test
    fun `clear removes the persisted first-attempt time too`() {
        val store = mutableMapOf<String, Long>()
        var now = 0L
        val tracker = SyncRetryTracker(
            maxDurationMs = 1_000L,
            nowMs = { now },
            loadFirstAttempt = { key -> store[key] },
            saveFirstAttempt = { key, ts -> store[key] = ts },
            clearFirstAttempt = { key -> store.remove(key) }
        )

        assertFalse(tracker.recordAttemptAndShouldGiveUp("hash1"))
        tracker.clear("hash1")
        assertFalse(store.containsKey("hash1"))

        now = 1_500L
        // A fresh tracker (simulating a restart) sees no persisted entry, so this is a genuinely
        // new first attempt rather than an immediate give-up.
        val restarted = SyncRetryTracker(
            maxDurationMs = 1_000L,
            nowMs = { now },
            loadFirstAttempt = { key -> store[key] },
            saveFirstAttempt = { key, ts -> store[key] = ts },
            clearFirstAttempt = { key -> store.remove(key) }
        )
        assertFalse(restarted.recordAttemptAndShouldGiveUp("hash1"))
    }
}
