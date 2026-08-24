package com.stablechannels.app.util

/**
 * Chain-freshness rule for stability payments (see #243).
 *
 * A stability payment may only be sent when LDK completed a Lightning-wallet chain sync
 * within [Constants.STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS]. Paying on a stale chain tip
 * understates outbound HTLC expiry, which LDK later force-closes on.
 *
 * Keep in sync with `lightning_sync_is_fresh` in `src/stable.rs` and `StabilityFreshness`
 * in the iOS app.
 */
object StabilityFreshness {

    /**
     * True when [latestSyncTimestampSecs] exists, is not in the future, and is at most
     * [maxAgeSecs] old (exactly [maxAgeSecs] old is accepted). A missing timestamp means
     * the wallet has never synced; a future one means clock skew — both block the send.
     */
    fun isFresh(
        latestSyncTimestampSecs: Long?,
        nowSecs: Long,
        maxAgeSecs: Long = Constants.STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS
    ): Boolean {
        if (latestSyncTimestampSecs == null) return false
        if (latestSyncTimestampSecs > nowSecs) return false
        return nowSecs - latestSyncTimestampSecs <= maxAgeSecs
    }

    /** Age of the last sync in seconds, or null when missing or in the future. */
    fun syncAgeSecs(latestSyncTimestampSecs: Long?, nowSecs: Long): Long? {
        if (latestSyncTimestampSecs == null || latestSyncTimestampSecs > nowSecs) return null
        return nowSecs - latestSyncTimestampSecs
    }
}
