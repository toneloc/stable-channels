package com.stablechannels.app.push

import android.content.Context

/**
 * The push direction the LSP sends before it splices its own funds into the channel. Both sides
 * sign the splice, so the node has to be online for the whole negotiation; the LSP sends it even
 * when the phone is already connected, because the app gets no LDK event until `splice_init`
 * arrives and would otherwise stop the node a few seconds after backgrounding.
 */
const val LSP_SPLICE_IN_DIRECTION = "lsp_splice_in"

/** Where a push goes once it reaches the phone. */
internal enum class PushRoute {
    /** The running app holds its own node for the splice. */
    HOLD_IN_APP,
    /**
     * The running app or the background service already owns the node; it picks the push up on its
     * next cycle.
     */
    FLAG_FOR_APP,
    /** Nothing owns the node: the background service boots one for the push. */
    START_SERVICE,
}

/**
 * Routes a push by who owns the node. A splice push for a running app becomes a hold, since the
 * app's next stability cycle would come too late to keep the node up; any other push to a running
 * owner is only flagged, as before.
 */
internal fun routePush(
    direction: String,
    mainAppActive: Boolean,
    nodeOwnedInProcess: Boolean,
    serviceRunning: Boolean,
    appCanHold: Boolean,
): PushRoute =
    when {
        (mainAppActive || nodeOwnedInProcess) &&
            direction == LSP_SPLICE_IN_DIRECTION &&
            appCanHold -> PushRoute.HOLD_IN_APP
        mainAppActive || nodeOwnedInProcess || serviceRunning -> PushRoute.FLAG_FOR_APP
        else -> PushRoute.START_SERVICE
    }

/**
 * In-process bridge from [FCMService] to the running app. The app registers while it owns the node;
 * the push handler asks it to hold the node for an LSP splice.
 */
object LspSpliceHoldBridge {
    @Volatile var holder: (() -> Unit)? = null

    val canHold: Boolean
        get() = holder != null

    /** Asks the app to hold its node; false when no app is registered to do so. */
    fun requestHold(): Boolean {
        val holder = holder ?: return false
        holder()
        return true
    }
}

/**
 * When the LSP last pushed for a splice of its own. It pushes before every one, and again every few
 * minutes while its call is unanswered, so a splice negotiated inside this window with no Move of
 * the user's in flight is the LSP's: a Move the app lost track of must not be given its
 * transaction.
 */
object LspSpliceExpectation {
    const val WINDOW_MS = 10 * 60 * 1000L
    private const val PREFS_NAME = "lsp_splice"
    private const val KEY_PUSHED_AT_MS = "pushed_at_ms"

    fun note(context: Context, nowMs: Long = System.currentTimeMillis()) {
        context
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putLong(KEY_PUSHED_AT_MS, nowMs)
            .apply()
    }

    fun isExpected(context: Context, nowMs: Long = System.currentTimeMillis()): Boolean =
        expectedWithin(
            context
                .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .getLong(KEY_PUSHED_AT_MS, 0L),
            nowMs,
        )

    /** From the push until the window ends; a clock set back before the push expects nothing. */
    internal fun expectedWithin(pushedAtMs: Long, nowMs: Long): Boolean =
        pushedAtMs > 0L && nowMs >= pushedAtMs && nowMs - pushedAtMs < WINDOW_MS
}

/**
 * When the background wait for an LSP splice ends: at a failed negotiation, at the lock plus a
 * settle window for a stability payment that may follow it, or at the deadline.
 */
internal class LspSpliceWait(startMs: Long, waitMs: Long, private val settleMs: Long) {
    enum class Outcome {
        TIMED_OUT,
        LOCKED,
        FAILED,
    }

    private val deadlineMs = startMs + waitMs
    private var lockedAtMs: Long? = null
    private var failed = false

    /** A lock is the outcome once seen; a failure after it belongs to some later negotiation. */
    val outcome: Outcome
        get() =
            when {
                lockedAtMs != null -> Outcome.LOCKED
                failed -> Outcome.FAILED
                else -> Outcome.TIMED_OUT
            }

    fun onLocked(nowMs: Long) {
        if (lockedAtMs == null) lockedAtMs = nowMs
    }

    fun onFailed() {
        failed = true
    }

    fun isDone(nowMs: Long): Boolean =
        failed || nowMs >= deadlineMs || lockedAtMs?.let { nowMs >= it + settleMs } == true
}
