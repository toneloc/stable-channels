package com.stablechannels.app.services

import android.content.ContentValues
import android.database.sqlite.SQLiteDatabase
import androidx.core.database.getLongOrNull
import androidx.core.database.sqlite.transaction
import org.lightningdevkit.ldknode.Event
import org.lightningdevkit.ldknode.OutPoint

/**
 * A splice the LSP made into the channel: negotiated on it with no Move of the user's own claiming
 * it. Kept apart from the user's history, so it never shows as something received.
 */
data class LspSplice(
    val txid: String,
    val userChannelId: String,
    val channelId: String,
    /** "negotiated" until the channel is ready on the new funding, then "locked". */
    val status: String,
    val valueBeforeSats: Long?,
    val valueAfterSats: Long?,
    val createdAt: Long,
    val lockedAt: Long?,
) {
    /** What the LSP added, once the channel's value is known on both sides of the splice. */
    val addedSats: Long?
        get() =
            if (valueBeforeSats != null && valueAfterSats != null) valueAfterSats - valueBeforeSats
            else null
}

/** A channel's value as the node listed it, with the funding it was on at the time. */
data class ChannelValue(val fundingTxid: String?, val valueSats: Long)

/** Durable operation identity and event receipt, shared by the foreground and push service. */
object SpliceEventRecorder {
    /** A negotiated LSP splice older than this no longer holds the books repair back. */
    const val LSP_SPLICE_PENDING_MAX_AGE_SECS = 24 * 60 * 60L

    internal fun createTables(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS splice_operations (
                payment_row_id INTEGER PRIMARY KEY,
                user_channel_id TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                previous_funding_txid TEXT
            )
        """
        )
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS splice_events (
                event_key TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                user_channel_id TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                txid TEXT,
                vout INTEGER,
                payment_row_id INTEGER
            )
        """
        )
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS lsp_splices (
                txid TEXT PRIMARY KEY,
                user_channel_id TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                status TEXT NOT NULL,
                value_before_sats INTEGER,
                value_after_sats INTEGER,
                created_at INTEGER NOT NULL,
                locked_at INTEGER
            )
        """
        )
    }

    /** Called in the same transaction that creates the pending history entry, before LDK. */
    internal fun track(
        db: SQLiteDatabase,
        rowId: Long,
        userChannelId: String,
        channelId: String,
        previousFundingTxid: String?,
    ) {
        require(userChannelId.isNotBlank() && channelId.isNotBlank())
        db.insertOrThrow(
            "splice_operations",
            null,
            ContentValues().apply {
                put("payment_row_id", rowId)
                put("user_channel_id", userChannelId)
                put("channel_id", channelId)
                put("previous_funding_txid", previousFundingTxid)
            },
        )
    }

    /**
     * Returns false for other event types; true means the event is committed and safe to
     * acknowledge. Unmatched events are kept but never reassigned to a later operation.
     *
     * A negotiation no operation claims is recorded as the LSP's splice, and the ready event on its
     * funding locks it. With [lspSplice] the caller knows the splice is the LSP's (the LSP pushed
     * for it and nothing of the user's is in flight), so a Move still waiting for its transaction
     * is not given this one; a replay of the user's own negotiation is still recognised by its
     * transaction. [channelValue] lists the channel as the node shows it now and [channelValueSeen]
     * as it was last listed before this event; the value before the splice comes from whichever
     * still shows another funding, the value after from the live listing on the new one, so the
     * LSP's amount is known.
     */
    fun record(
        service: DatabaseService,
        event: Event,
        lspSplice: Boolean = false,
        channelValue: (String, String) -> ChannelValue? = { _, _ -> null },
        channelValueSeen: (String, String) -> ChannelValue? = { _, _ -> null },
        nowEpochSecs: Long = System.currentTimeMillis() / 1000,
        fundingForReady: (Event.ChannelReady) -> OutPoint?,
    ): Boolean {
        val kind: String
        val channelId: String
        val userChannelId: String
        val funding: OutPoint?
        when (event) {
            is Event.SpliceNegotiated -> {
                kind = "negotiated"
                channelId = event.channelId
                userChannelId = event.userChannelId
                funding = event.newFundingTxo
            }
            is Event.ChannelReady -> {
                kind = "ready"
                channelId = event.channelId
                userChannelId = event.userChannelId
                // A stale or early ready event may lack a live funding lookup; keep its identity
                // and captured operation so foreground recovery can retry.
                funding = event.fundingTxo ?: fundingForReady(event)
            }
            else -> return false
        }
        val db = service.writableDatabase
        var lspChange: LspSplice? = null
        db.transaction {
            // Deduplicate from the event itself, never a changing live snapshot, so an
            // outpoint-less legacy ready replay cannot capture a newer operation.
            val identityTxid =
                when (event) {
                    is Event.ChannelReady -> event.fundingTxo?.txid
                    is Event.SpliceNegotiated -> event.newFundingTxo.txid
                    else -> null
                }
            val key = "$kind:$userChannelId:$channelId:${identityTxid ?: "unknown"}"
            val seen =
                rawQuery("SELECT 1 FROM splice_events WHERE event_key = ?", arrayOf(key)).use {
                    it.moveToFirst()
                }
            if (!seen) {
                val rowId =
                    findOperation(
                        db,
                        kind,
                        userChannelId,
                        channelId,
                        funding?.txid,
                        matchPending = !lspSplice,
                    )
                insertOrThrow(
                    "splice_events",
                    null,
                    ContentValues().apply {
                        put("event_key", key)
                        put("event_type", kind)
                        put("user_channel_id", userChannelId)
                        put("channel_id", channelId)
                        put("payment_row_id", rowId)
                    },
                )
                if (kind == "negotiated" && rowId == null && funding != null) {
                    // Nothing of the user's claims the splice, so it is the LSP's.
                    insertWithOnConflict(
                        "lsp_splices",
                        null,
                        ContentValues().apply {
                            put("txid", funding.txid)
                            put("user_channel_id", userChannelId)
                            put("channel_id", channelId)
                            put("status", "negotiated")
                            put(
                                "value_before_sats",
                                valueBefore(
                                    channelValue(userChannelId, channelId),
                                    channelValueSeen(userChannelId, channelId),
                                    funding.txid,
                                ),
                            )
                            put("created_at", nowEpochSecs)
                        },
                        SQLiteDatabase.CONFLICT_IGNORE,
                    )
                    lspChange = lspSplice(this, funding.txid)
                }
            }
            if (funding != null) {
                update(
                    "splice_events",
                    ContentValues().apply {
                        put("txid", funding.txid)
                        put("vout", funding.vout.toLong())
                    },
                    "event_key = ? AND txid IS NULL",
                    arrayOf(key),
                )
            }
            if (kind == "ready" && funding != null) {
                val locked =
                    update(
                        "lsp_splices",
                        ContentValues().apply {
                            put("status", "locked")
                            put(
                                "value_after_sats",
                                channelValue(userChannelId, channelId)
                                    ?.takeIf { it.fundingTxid == funding.txid }
                                    ?.valueSats,
                            )
                            put("locked_at", nowEpochSecs)
                        },
                        "txid = ? AND status = 'negotiated'",
                        arrayOf(funding.txid),
                    )
                if (locked == 1) lspChange = lspSplice(this, funding.txid)
            }
            recoverAssignments(service)
        }
        lspChange?.let { splice ->
            AuditService.log(
                if (splice.status == "locked") "LSP_SPLICE_LOCKED" else "LSP_SPLICE_NEGOTIATED",
                mapOf(
                    "channel_id" to splice.channelId,
                    "user_channel_id" to splice.userChannelId,
                    "txid" to splice.txid,
                    "value_before_sats" to splice.valueBeforeSats,
                    "value_after_sats" to splice.valueAfterSats,
                    "added_sats" to splice.addedSats,
                ),
            )
        }
        return true
    }

    /**
     * The channel's value before the splice on [newTxid]. A 0-conf splice can lock before the app
     * handles its negotiation, so a listing counts only while it shows another funding: the live
     * one first, else the last one seen before the event. Null when neither does, rather than the
     * new size as the old.
     */
    internal fun valueBefore(live: ChannelValue?, seen: ChannelValue?, newTxid: String): Long? =
        listOf(live, seen).firstNotNullOfOrNull { value ->
            value?.takeIf { it.fundingTxid != newTxid }?.valueSats
        }

    /** The LSP splice on [txid], if one was recorded. */
    fun lspSplice(service: DatabaseService, txid: String): LspSplice? =
        lspSplice(service.readableDatabase, txid)

    private fun lspSplice(db: SQLiteDatabase, txid: String): LspSplice? =
        db.rawQuery(
                """
            SELECT txid, user_channel_id, channel_id, status, value_before_sats, value_after_sats,
                   created_at, locked_at
            FROM lsp_splices WHERE txid = ?
        """,
                arrayOf(txid),
            )
            .use { c ->
                if (!c.moveToFirst()) null
                else
                    LspSplice(
                        txid = c.getString(0),
                        userChannelId = c.getString(1),
                        channelId = c.getString(2),
                        status = c.getString(3),
                        valueBeforeSats = c.getLongOrNull(4),
                        valueAfterSats = c.getLongOrNull(5),
                        createdAt = c.getLong(6),
                        lockedAt = c.getLongOrNull(7),
                    )
            }

    /**
     * Whether the LSP negotiated a splice in the last [maxAgeSecs] that has not locked: the channel
     * is changing under the books, which must not be repaired against it meanwhile.
     */
    fun hasPendingLspSplice(
        service: DatabaseService,
        nowEpochSecs: Long,
        maxAgeSecs: Long = LSP_SPLICE_PENDING_MAX_AGE_SECS,
    ): Boolean =
        service.readableDatabase
            .rawQuery(
                "SELECT 1 FROM lsp_splices WHERE status = 'negotiated' AND created_at > ? LIMIT 1",
                arrayOf((nowEpochSecs - maxAgeSecs).toString()),
            )
            .use { it.moveToFirst() }

    /** With [matchPending] false only an operation already carrying the transaction is found. */
    private fun findOperation(
        db: SQLiteDatabase,
        kind: String,
        userChannelId: String,
        channelId: String,
        txid: String?,
        matchPending: Boolean = true,
    ): Long? {
        // Exact txid matches also recognize terminal replays, without reviving their rows.
        val exact =
            db.rawQuery(
                    """
            SELECT p.id FROM payments p LEFT JOIN splice_operations s ON s.payment_row_id = p.id
            WHERE p.txid = ? AND p.payment_type IN ('splice_in','splice_out')
              AND (s.user_channel_id IS NULL OR s.user_channel_id = ?)
            LIMIT 2
        """,
                    arrayOf(txid.orEmpty(), userChannelId),
                )
                .use { c ->
                    buildList { while (c.moveToNext()) add(c.getLong(0)) }
                }
        if (exact.isNotEmpty()) return exact.singleOrNull()
        if (!matchPending) return null

        // LDK keeps the channel ID across a splice, so a ready event must also prove funding
        // changed from the saved outpoint, or an initial channel-ready replay would look like a
        // splice.
        val readyClause =
            if (kind == "ready")
                "AND s.previous_funding_txid IS NOT NULL AND (? = '' OR s.previous_funding_txid != ?)"
            else "AND (s.previous_funding_txid IS NULL OR s.previous_funding_txid != ?)"
        val trackedArgs = buildList {
            add(userChannelId)
            add(channelId)
            if (kind == "ready") add(txid.orEmpty())
            add(txid.orEmpty())
        }
            .toTypedArray()
        val tracked =
            db.rawQuery(
                    """
            SELECT p.id FROM payments p JOIN splice_operations s ON s.payment_row_id = p.id
            WHERE p.status = 'pending' AND p.txid IS NULL
              AND s.user_channel_id = ? AND s.channel_id = ? $readyClause
            LIMIT 2
        """,
                    trackedArgs,
                )
                .use { c ->
                    buildList { while (c.moveToNext()) add(c.getLong(0)) }
                }
        if (tracked.isNotEmpty()) return tracked.singleOrNull()

        if (kind != "negotiated") return null

        // Upgrade path for rows created before operation identity was persisted: keep the
        // conservative single-recent-candidate rule, scoped to the saved channel.
        val legacy =
            db.rawQuery(
                    """
            SELECT p.id FROM payments p
            WHERE p.payment_type IN ('splice_in','splice_out') AND p.status = 'pending'
              AND p.txid IS NULL AND p.created_at >= ?
              AND NOT EXISTS (SELECT 1 FROM splice_operations s WHERE s.payment_row_id = p.id)
              AND EXISTS (SELECT 1 FROM channels c WHERE c.user_channel_id = ?
                          AND c.channel_id = ?)
            LIMIT 2
        """,
                    arrayOf(
                        (System.currentTimeMillis() / 1000 -
                                DatabaseService.PENDING_SPLICE_WITHOUT_TXID_TIMEOUT_SECS)
                            .toString(),
                        userChannelId,
                        channelId,
                    ),
                )
                .use { c -> buildList { while (c.moveToNext()) add(c.getLong(0)) } }
        return legacy.singleOrNull()
    }

    /** Resolve saved ChannelReady events against the same channel after node startup. */
    fun recoverReadyEvents(service: DatabaseService, funding: (String, String) -> OutPoint?) {
        service.writableDatabase.transaction {
            val events =
                rawQuery(
                        """
                SELECT event_key, user_channel_id, channel_id FROM splice_events
                WHERE event_type = 'ready' AND txid IS NULL
            """,
                        null,
                    )
                    .use { c ->
                        buildList {
                            while (c.moveToNext()) add(
                                Triple(c.getString(0), c.getString(1), c.getString(2))
                            )
                        }
                    }
            for ((key, userChannelId, channelId) in events) {
                val txo = funding(userChannelId, channelId) ?: continue
                update(
                    "splice_events",
                    ContentValues().apply {
                        put("txid", txo.txid)
                        put("vout", txo.vout.toLong())
                    },
                    "event_key = ? AND txid IS NULL",
                    arrayOf(key),
                )
            }
            execSQL(
                """
                UPDATE splice_events SET payment_row_id = NULL
                WHERE event_type = 'ready' AND EXISTS (
                    SELECT 1 FROM splice_operations s WHERE s.payment_row_id = splice_events.payment_row_id
                      AND s.previous_funding_txid = splice_events.txid
                )
            """
            )
            recoverAssignments(service)
        }
    }

    /**
     * Retries only captured identities (after a restart or txid collision); the payment's creation
     * time bypasses the pre-negotiation timeout once an event proved negotiation. Failed rows stay
     * terminal.
     */
    fun recoverAssignments(service: DatabaseService) {
        service.writableDatabase.transaction {
            val pending =
                rawQuery(
                        """
                SELECT e.txid, p.id, p.created_at FROM splice_events e
                JOIN payments p ON p.id = e.payment_row_id
                LEFT JOIN splice_operations s ON s.payment_row_id = p.id
                WHERE p.status = 'pending' AND p.txid IS NULL AND e.txid IS NOT NULL
                  AND (e.event_type != 'ready' OR s.previous_funding_txid != e.txid)
            """,
                        null,
                    )
                    .use { c ->
                        buildList {
                            while (c.moveToNext()) add(
                                Triple(c.getString(0), c.getLong(1), c.getLong(2))
                            )
                        }
                    }
            for ((txid, rowId, createdAt) in pending) {
                service.assignPendingSpliceTxid(txid, rowId, createdAt)
            }
        }
    }
}
