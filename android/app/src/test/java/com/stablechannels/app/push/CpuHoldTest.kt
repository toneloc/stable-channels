package com.stablechannels.app.push

import android.content.Context
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowPowerManager

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CpuHoldTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    @Test
    fun `a hold keeps the cpu awake with a partial wake lock until it is released`() {
        val hold = CpuHold(context, "stablechannels:test")
        assertFalse(hold.isHeld)

        hold.hold(forMs = 60_000L)
        val lock = ShadowPowerManager.getLatestWakeLock()
        assertTrue(hold.isHeld)
        assertTrue(lock.isHeld)
        assertEquals("stablechannels:test", shadowOf(lock).tag)

        hold.release()
        assertFalse(hold.isHeld)
        assertFalse(lock.isHeld)
        // A release with nothing held is not an error.
        hold.release()
        assertFalse(hold.isHeld)
    }

    @Test
    fun `a second hold does not need a second release`() {
        val hold = CpuHold(context, "stablechannels:test")
        hold.hold(forMs = 1_000L)
        hold.hold(forMs = 60_000L)
        hold.release()
        assertFalse(hold.isHeld)
    }
}
