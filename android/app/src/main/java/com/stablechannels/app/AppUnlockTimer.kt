package com.stablechannels.app

import android.os.SystemClock

/**
 * Tracks how long the app was away, for App Unlock. Uses the time since boot, which the user can't
 * change in Settings.
 */
internal class AppUnlockTimer(private val now: () -> Long = SystemClock::elapsedRealtime) {
    private var pausedAt: Long? = null

    fun onPause() {
        pausedAt = now()
    }

    /**
     * True when the app was away longer than [GRACE_MS]. False before the first pause: the lock set
     * in onCreate covers the launch.
     */
    fun awayTooLong(): Boolean = pausedAt?.let { now() - it > GRACE_MS } ?: false

    companion object {
        const val GRACE_MS = 5000L
    }
}
