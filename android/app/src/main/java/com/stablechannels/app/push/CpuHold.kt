package com.stablechannels.app.push

import android.content.Context
import android.os.PowerManager

/**
 * Keeps the CPU awake while the node works for the LSP's splice. Once the push's own wake-up ends,
 * a phone with its screen off suspends, and a suspended node answers no message: the LSP's call
 * fails, and a wait on the phone sits until something else wakes the device. The hold is bounded,
 * so a wait that never ends cannot keep the phone awake either.
 */
class CpuHold(context: Context, tag: String) {
    private val lock =
        (context.getSystemService(Context.POWER_SERVICE) as PowerManager)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, tag)
            .apply { setReferenceCounted(false) }

    val isHeld: Boolean
        get() = lock.isHeld

    /** Holds the CPU for at most [forMs]; a second hold restarts the time. */
    fun hold(forMs: Long) {
        lock.acquire(forMs)
    }

    /** Lets the CPU sleep again; nothing happens when no hold is left. */
    fun release() {
        if (lock.isHeld) lock.release()
    }
}
