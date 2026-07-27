import Foundation
import os.log

/// Store for recently-processed WebSocket txids with TTL and memory cap.
@MainActor
final class ProcessedTxStore {
    var entries: [String: Date] = [:]
    let ttl: TimeInterval
    let maxEntries: Int

    private var lastPurgeTime: Date = .distantPast
    private let purgeInterval: TimeInterval = 300 // 5 minutes
    private let logger = Logger(subsystem: "com.stablechannels", category: "dedup")

    init(ttl: TimeInterval = 900, maxEntries: Int = 500) {
        self.ttl = ttl
        self.maxEntries = maxEntries
    }

    // MARK: - Public API

    /// Returns `true` if the key was seen within the TTL window.
    func isRecentlyProcessed(_ key: String) -> Bool {
        guard let lastSeen = entries[key] else { return false }
        return Date().timeIntervalSince(lastSeen) < ttl
    }

    /// Record a processed key and enforce the memory cap.
    func recordProcessedTx(_ key: String) {
        entries[key] = Date()
        enforceCap()
        purgeExpiredIfDue()
    }

    // MARK: - Internal (testable)

    /// Enforce a hard entry cap by evicting the oldest 20% of entries.
    func enforceCap() {
        guard entries.count > maxEntries else { return }
        let evictCount = maxEntries / 5 // remove ~20%
        let sorted = entries.sorted { $0.value < $1.value }
        for (key, _) in sorted.prefix(evictCount) {
            entries.removeValue(forKey: key)
        }
        logger.debug("Evicted \(evictCount) oldest entries (count was \(sorted.count))")
    }

    /// Purge expired entries at most every 5 minutes.
    func purgeExpiredIfDue() {
        let now = Date()
        if now.timeIntervalSince(lastPurgeTime) < purgeInterval {
            return
        }
        let cutoff = now.timeIntervalSince1970 - ttl
        let before = entries.count
        entries = entries.filter { _, date in
            date.timeIntervalSince1970 > cutoff
        }
        let removed = before - entries.count
        if removed > 0 {
            logger.debug("Purged \(removed) expired entries")
        }
        lastPurgeTime = now
    }

    /// Current entry count (for tests and metrics).
    var count: Int { entries.count }
}
