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
 * Whether the app's heartbeat still means the app owns the node. The app runs in one process, so a
 * heartbeat written before this process started was left by a process that died without clearing
 * it, and no longer means the app owns the node. A heartbeat from the second the process started
 * still counts, as before.
 */
internal fun heartbeatIsLive(
    lastActiveSecs: Long,
    nowSecs: Long,
    processStartSecs: Long,
    thresholdSecs: Long,
): Boolean = lastActiveSecs >= processStartSecs && nowSecs - lastActiveSecs < thresholdSecs

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
 * transaction, unless it is a splice-out and the channel turns out smaller once ready.
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

    /**
     * Whether a splice is taken to be the LSP's: while its push is handled, or inside the window
     * after it, and never while a Move of the user's is in flight. The one rule the app and the
     * background both use.
     */
    fun expectsLspSplice(
        context: Context,
        handlingSplicePush: Boolean = false,
        moveInFlight: Boolean = false,
    ): Boolean = !moveInFlight && (handlingSplicePush || isExpected(context))

    /** From the push until the window ends; a clock set back before the push expects nothing. */
    internal fun expectedWithin(pushedAtMs: Long, nowMs: Long): Boolean =
        pushedAtMs > 0L && nowMs >= pushedAtMs && nowMs - pushedAtMs < WINDOW_MS
}

/**
 * When the background wait for an LSP splice ends: at the lock plus a settle window for a stability
 * payment that may follow it, or at the deadline. A failed negotiation is only reported: its event
 * names no funding, so it may be one left over from an earlier attempt, with the LSP's splice still
 * to come.
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
        nowMs >= deadlineMs || lockedAtMs?.let { nowMs >= it + settleMs } == true
}

/**
 * The LSP's splice has locked once its channel is ready on new funding that is not the user's own
 * Move. A replayed ready event, or one for another channel, leaves the LSP channel's funding as it
 * was. The user's Move is a splice of the same channel, so it moves the funding too, to a
 * transaction the app has recorded as its own.
 */
internal fun lspSpliceLocked(
    fundingBeforeTxid: String?,
    fundingNowTxid: String?,
    isOwnMove: (String) -> Boolean,
): Boolean =
    fundingNowTxid != null && fundingNowTxid != fundingBeforeTxid && !isOwnMove(fundingNowTxid)
