package com.stablechannels.app.services

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull

/** Outcome of one payment-confirmation pass. */
sealed interface ConfirmationPollResult {
    /** The pass ran; [failedLookups] transactions could not be checked against the chain. */
    data class Completed(val failedLookups: Int = 0) : ConfirmationPollResult

    /** The chain tip could not be fetched, so no confirmations were checked. */
    data object ChainTipUnavailable : ConfirmationPollResult

    /** Payment history is not open yet. */
    data object DatabaseUnavailable : ConfirmationPollResult

    /** A manual refresh hit its deadline (waiting for another pass included) and was cancelled. */
    data object TimedOut : ConfirmationPollResult
}

data class ConfirmationPollUpdate(
    val sequence: Long,
    val result: ConfirmationPollResult,
)

/** User-facing message for a manual refresh, or null when the refresh fully succeeded. */
fun ConfirmationPollResult.refreshErrorMessage(): String? =
    when (this) {
        is ConfirmationPollResult.Completed ->
            when (failedLookups) {
                0 -> null
                1 -> "Couldn't check 1 transaction. Pull to try again."
                else -> "Couldn't check $failedLookups transactions. Pull to try again."
            }
        ConfirmationPollResult.ChainTipUnavailable ->
            "Couldn't reach the block explorer. Pull to try again."
        ConfirmationPollResult.DatabaseUnavailable -> "Payment history is unavailable right now."
        ConfirmationPollResult.TimedOut ->
            "Checking confirmations took too long. Pull to try again."
    }

/**
 * Serializes payment-confirmation passes. Automatic polls keep their existing semantics (throttled
 * unless forced, skipped while another pass runs); a manual [refresh] waits for any in-flight pass
 * and then runs a fresh one, so its result reflects a pass that started after the request.
 */
class ConfirmationRefreshCoordinator(
    private val minIntervalMs: Long = DEFAULT_MIN_INTERVAL_MS,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val onResult: (ConfirmationPollResult) -> Unit = {},
    /** Runs one pass; `manual` is true for [refresh], false for [pollIfIdle]. */
    private val pass: suspend (manual: Boolean) -> ConfirmationPollResult,
) {
    private val mutex = Mutex()
    @Volatile private var lastCompletedPassStartedAtMs = 0L

    /** Automatic poll. Returns null when throttled or when another pass is already running. */
    suspend fun pollIfIdle(force: Boolean): ConfirmationPollResult? {
        val now = nowMs()
        if (!force && (now - lastCompletedPassStartedAtMs) < minIntervalMs) return null
        if (!mutex.tryLock()) return null
        return try {
            runPass(now, manual = false)
        } finally {
            mutex.unlock()
        }
    }

    /**
     * Manual refresh. Suspends until a pass started after this call has completed, or returns
     * [ConfirmationPollResult.TimedOut] once [deadlineMs] has elapsed. The deadline covers waiting
     * for an in-flight pass as well as running our own; on expiry the pass is cancelled (its
     * requests with it) and the lock is released.
     */
    suspend fun refresh(deadlineMs: Long = MANUAL_REFRESH_DEADLINE_MS): ConfirmationPollResult {
        val result =
            withTimeoutOrNull(deadlineMs) { mutex.withLock { runPass(nowMs(), manual = true) } }
                ?: ConfirmationPollResult.TimedOut
        if (result == ConfirmationPollResult.TimedOut) onResult(result)
        return result
    }

    private suspend fun runPass(startedAtMs: Long, manual: Boolean): ConfirmationPollResult {
        val result = pass(manual)
        // Same as before: only a pass that reached the chain tip advances the throttle.
        if (result is ConfirmationPollResult.Completed) {
            lastCompletedPassStartedAtMs = startedAtMs
        }
        onResult(result)
        return result
    }

    companion object {
        const val DEFAULT_MIN_INTERVAL_MS = 15_000L
        const val MANUAL_REFRESH_DEADLINE_MS = 20_000L
    }
}
