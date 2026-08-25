package com.stablechannels.app

import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.StabilityFreshness
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Chain-freshness rule for stability payments (see #243): a stability payment may only be
 * sent when LDK completed a Lightning-wallet sync within the last 120 seconds.
 */
class StabilityFreshnessTest {

    private val now = 1_000_000L
    private val maxAge = Constants.STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS

    @Test
    fun `missing timestamp blocks the send`() {
        assertFalse(StabilityFreshness.isFresh(null, now))
    }

    @Test
    fun `future timestamp blocks the send`() {
        assertFalse(StabilityFreshness.isFresh(now + 1, now))
    }

    @Test
    fun `timestamp older than the window blocks the send`() {
        assertFalse(StabilityFreshness.isFresh(now - maxAge - 1, now))
    }

    @Test
    fun `timestamp exactly at the window boundary is accepted`() {
        assertTrue(StabilityFreshness.isFresh(now - maxAge, now))
    }

    @Test
    fun `fresh timestamp is accepted`() {
        assertTrue(StabilityFreshness.isFresh(now, now))
        assertTrue(StabilityFreshness.isFresh(now - 1, now))
    }

    @Test
    fun `sync age is null for missing or future timestamps`() {
        assertNull(StabilityFreshness.syncAgeSecs(null, now))
        assertNull(StabilityFreshness.syncAgeSecs(now + 1, now))
    }

    @Test
    fun `sync age reports seconds since the last sync`() {
        assertEquals(0L, StabilityFreshness.syncAgeSecs(now, now))
        assertEquals(90L, StabilityFreshness.syncAgeSecs(now - 90, now))
    }
}
