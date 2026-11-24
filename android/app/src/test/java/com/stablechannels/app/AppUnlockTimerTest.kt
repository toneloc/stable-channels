package com.stablechannels.app

import java.time.Duration
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowSystemClock

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class AppUnlockTimerTest {
    private var clock = 1_000_000L
    private val timer = AppUnlockTimer { clock }

    @Test
    fun noLockBeforeTheFirstPause() {
        clock += 60_000
        assertFalse(timer.awayTooLong())
    }

    @Test
    fun locksAfterFiveSeconds() {
        timer.onPause()
        clock += 5_001
        assertTrue(timer.awayTooLong())
    }

    @Test
    fun fiveSecondsOrLessDoesNotLock() {
        timer.onPause()
        clock += 5_000
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

    @Test
    fun theDefaultClockCountsTimeAsleep() {
        // Only the boot clock moves during deep sleep here. A timer on the wall clock or the
        // uptime clock would miss these 6 s.
        val bootClockTimer = AppUnlockTimer()
        bootClockTimer.onPause()
        ShadowSystemClock.simulateDeepSleep(Duration.ofSeconds(6))
        assertTrue(bootClockTimer.awayTooLong())
    }
}
