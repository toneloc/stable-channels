package com.stablechannels.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppUnlockTimerTest {
    private var clock = 1_000_000L
    private val timer = AppUnlockTimer { clock }

    @Test
    fun noLockBeforeTheFirstPause() {
        clock += 60_000
        assertFalse(timer.awayTooLong())
    }

    @Test
    fun locksAfterTheGracePeriod() {
        timer.onPause()
        clock += AppUnlockTimer.GRACE_MS + 1
        assertTrue(timer.awayTooLong())
    }

    @Test
    fun aQuickReturnDoesNotLock() {
        timer.onPause()
        clock += AppUnlockTimer.GRACE_MS
        assertFalse(timer.awayTooLong())
    }

    @Test
    fun eachPauseRestartsTheTimer() {
        timer.onPause()
        clock += 60_000
        timer.onPause()
        clock += 1_000
        assertFalse(timer.awayTooLong())
    }
}
