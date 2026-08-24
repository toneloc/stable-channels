package com.stablechannels.app.services.websocket

import java.util.concurrent.ConcurrentHashMap

class ProcessedTxStore(
    private val ttlMs: Long = 900_000L,
    private val maxEntries: Int = 500
) {
    private val entries = ConcurrentHashMap<String, Long>()

    @Volatile
    private var lastPurgeAtMs: Long = 0L

    private val purgeIntervalMs: Long = 300_000L

    fun isRecentlyProcessed(key: String): Boolean {
        val seenAt = entries[key] ?: return false
        return (System.currentTimeMillis() - seenAt) < ttlMs
    }

    fun recordProcessedTx(key: String) {
        entries[key] = System.currentTimeMillis()
        enforceCap()
        purgeExpiredIfDue()
    }

    fun count(): Int = entries.size

    private fun enforceCap() {
        if (entries.size <= maxEntries) {
            return
        }
        val evictCount = (maxEntries / 5).coerceAtLeast(1)
        val sortedOldest = entries.entries.sortedBy { it.value }
        sortedOldest.take(evictCount).forEach { (key, _) ->
            entries.remove(key)
        }
    }

    private fun purgeExpiredIfDue() {
        val now = System.currentTimeMillis()
        if ((now - lastPurgeAtMs) < purgeIntervalMs) {
            return
        }
        val cutoff = now - ttlMs
        entries.entries.removeIf { it.value <= cutoff }
        lastPurgeAtMs = now
    }
}
