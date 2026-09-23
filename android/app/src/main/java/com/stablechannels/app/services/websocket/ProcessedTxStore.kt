package com.stablechannels.app.services.websocket

class ProcessedTxStore(
    private val ttlMs: Long = 900_000L,
    private val maxEntries: Int = 500,
) {
    private val entries = LinkedHashMap<String, Long>()
    private val lock = Any()

    @Volatile private var lastPurgeAtMs: Long = 0L

    private val purgeIntervalMs: Long = 300_000L

    fun isRecentlyProcessed(key: String): Boolean {
        val seenAt = synchronized(lock) { entries[key] } ?: return false
        return (System.currentTimeMillis() - seenAt) < ttlMs
    }

    fun recordProcessedTx(key: String) {
        val now = System.currentTimeMillis()
        synchronized(lock) {
            entries.remove(key)
            entries[key] = now
            enforceCap()
            purgeExpiredIfDue(now)
        }
    }

    fun count(): Int = synchronized(lock) { entries.size }

    private fun enforceCap() {
        if (entries.size <= maxEntries) {
            return
        }
        val evictCount = (maxEntries / 5).coerceAtLeast(1)
        val it = entries.keys.iterator()
        var removed = 0
        while (it.hasNext() && removed < evictCount) {
            it.next()
            it.remove()
            removed++
        }
    }

    private fun purgeExpiredIfDue(now: Long) {
        if ((now - lastPurgeAtMs) < purgeIntervalMs) {
            return
        }
        val cutoff = now - ttlMs
        entries.values.removeIf { it <= cutoff }
        lastPurgeAtMs = now
    }
}
