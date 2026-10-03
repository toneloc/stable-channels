package com.stablechannels.app.services

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Outcome of one payment-confirmation pass. */
sealed interface ConfirmationPollResult {
    /** The pass ran; [failedLookups] transactions could not be checked against the chain. */
    data class Completed(val failedLookups: Int = 0) : ConfirmationPollResult

    /** The chain tip could not be fetched, so no confirmations were checked. */
    data object ChainTipUnavailable : ConfirmationPollResult

    /** Payment history is not open yet. */
    data object DatabaseUnavailable : ConfirmationPollResult
}

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
    }

/**
 * Serializes payment-confirmation passes. Automatic polls keep their existing semantics (throttled
 * unless forced, skipped while another pass runs); a manual [refresh] waits for any in-flight pass
 * and then runs a fresh one, so its result reflects a pass that started after the request.
 */
class ConfirmationRefreshCoordinator(
    private val minIntervalMs: Long = DEFAULT_MIN_INTERVAL_MS,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val pass: suspend () -> ConfirmationPollResult,
) {
    private val mutex = Mutex()
    @Volatile private var lastCompletedPassStartedAtMs = 0L

    /** Automatic poll. Returns null when throttled or when another pass is already running. */
    suspend fun pollIfIdle(force: Boolean): ConfirmationPollResult? {
        val now = nowMs()
        if (!force && (now - lastCompletedPassStartedAtMs) < minIntervalMs) return null
        if (!mutex.tryLock()) return null
        return try {
            runPass(now)
        } finally {
            mutex.unlock()
        }
    }

    /** Manual refresh. Suspends until a pass started after this call has completed. */
    suspend fun refresh(): ConfirmationPollResult = mutex.withLock { runPass(nowMs()) }

    private suspend fun runPass(startedAtMs: Long): ConfirmationPollResult {
        val result = pass()
        // Same as before: only a pass that reached the chain tip advances the throttle.
        if (result is ConfirmationPollResult.Completed) {
            lastCompletedPassStartedAtMs = startedAtMs
        }
        return result
    }

    companion object {
        const val DEFAULT_MIN_INTERVAL_MS = 15_000L
    }
}
