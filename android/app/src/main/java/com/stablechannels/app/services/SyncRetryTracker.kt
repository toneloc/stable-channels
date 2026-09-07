package com.stablechannels.app.services

/**
 * Bounds how long AppState keeps retrying (and therefore blocking LDK's single-threaded event
 * queue on) the same unresolved signed trade-sync message. NodeService will not advance to the
 * next LDK event until the current one is acknowledged, so a trade result that can never commit
 * (e.g. the local channel row hasn't caught up yet) would otherwise retry forever and silently
 * block every event after it, including Event.ChannelClosed.
 *
 * The first-attempt timestamp is persisted via [loadFirstAttempt]/[saveFirstAttempt] (backed by
 * SharedPreferences in AppState), not just kept in memory. LDK durably persists an un-acked event
 * and redelivers it after the app process restarts (which Android can do at any time — screen
 * off, background app limits, low memory — well before 5 minutes of continuous foreground time
 * accumulates). An in-memory-only clock would reset on every restart and never actually give up.
 */
class SyncRetryTracker(
    private val maxDurationMs: Long = DEFAULT_MAX_DURATION_MS,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val loadFirstAttempt: (String) -> Long? = { null },
    private val saveFirstAttempt: (String, Long) -> Unit = { _, _ -> },
    private val clearFirstAttempt: (String) -> Unit = {}
) {
    private val firstAttemptAtMs = mutableMapOf<String, Long>()

    /** Records another attempt for [key]; returns true once [maxDurationMs] has elapsed since the first. */
    fun recordAttemptAndShouldGiveUp(key: String): Boolean {
        val now = nowMs()
        val firstAttempt = firstAttemptAtMs.getOrPut(key) {
            loadFirstAttempt(key) ?: now.also { saveFirstAttempt(key, it) }
        }
        if (now - firstAttempt >= maxDurationMs) {
            firstAttemptAtMs.remove(key)
            clearFirstAttempt(key)
            return true
        }
        return false
    }

    /** Clears tracking for [key] once it resolves definitively (applied, invalid, or duplicate). */
    fun clear(key: String) {
        firstAttemptAtMs.remove(key)
        clearFirstAttempt(key)
    }

    companion object {
        const val DEFAULT_MAX_DURATION_MS = 5 * 60 * 1000L
    }
}
