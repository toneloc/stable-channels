package com.stablechannels.app.services

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import com.stablechannels.app.models.*
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.HistoricalPrices
import java.io.File

data class PaymentPersistenceResult(
    val isNewPayment: Boolean,
    val backingSats: Long?
)

enum class TradeControlApplyStatus { APPLIED, DUPLICATE, INVALID, RETRY }

data class TradeControlApplyResult(
    val status: TradeControlApplyStatus,
    val localBackingSats: Long? = null,
    val peerBackingSats: Long? = null,
    val paymentId: String? = null,
    val action: String? = null,
    val allocationApplied: Boolean? = null
)

/** A backing update targeted a user_channel_id with no channels row. Callers can recreate the
 *  row from in-memory state and retry, unlike generic persistence failures. */
class MissingChannelRowException(userChannelId: String) :
    IllegalStateException("No channel row for user_channel_id=$userChannelId")

/** Durable marker for an in-flight outgoing stability payment (single row, id = 1).
 *  An empty paymentId means the keysend outcome is not yet known. */
data class PendingStabilitySend(
    val paymentId: String,
    val amountMsat: Long,
    val price: Double,
    val createdAt: Long
)

class DatabaseService(context: Context) : SQLiteOpenHelper(
    context,
    File(Constants.userDataDir(context), DB_FILENAME).absolutePath,
    null,
    DB_VERSION
) {
    companion object {
        private const val DB_FILENAME = "stablechannels.db"
        internal const val DB_VERSION = 3
    }

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("""
            CREATE TABLE IF NOT EXISTS channels (
                channel_id TEXT PRIMARY KEY,
                user_channel_id TEXT UNIQUE,
                expected_usd REAL DEFAULT 0,
                stable_sats INTEGER DEFAULT 0,
                note TEXT,
                receiver_sats INTEGER NOT NULL DEFAULT 0,
                latest_price REAL NOT NULL DEFAULT 0.0,
                sync_version INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s','now')),
                updated_at INTEGER DEFAULT (strftime('%s','now'))
            )
        """)

        db.execSQL("""
            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT,
                action TEXT NOT NULL,
                amount_usd REAL NOT NULL,
                amount_btc REAL NOT NULL,
                btc_price REAL NOT NULL,
                fee_usd REAL DEFAULT 0,
                payment_id TEXT,
                status TEXT DEFAULT 'pending',
                user_channel_id TEXT,
                trade_id TEXT,
                request_hash TEXT,
                request_payload TEXT,
                trade_payment_id TEXT,
                old_expected_usd REAL,
                new_expected_usd REAL,
                new_backing_sats INTEGER,
                quote_price REAL,
                fee_msat INTEGER NOT NULL DEFAULT 0,
                expires_at INTEGER,
                outcome TEXT,
                reason_code TEXT,
                uncertainty_reason TEXT,
                resolved_at INTEGER,
                created_at INTEGER DEFAULT (strftime('%s','now'))
            )
        """)

        db.execSQL("""
            CREATE TABLE IF NOT EXISTS payments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payment_id TEXT,
                payment_type TEXT NOT NULL,
                direction TEXT NOT NULL,
                amount_msat INTEGER NOT NULL,
                amount_usd REAL,
                btc_price REAL,
                counterparty TEXT,
                status TEXT DEFAULT 'pending',
                fee_msat INTEGER DEFAULT 0,
                txid TEXT,
                address TEXT,
                confirmations INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s','now'))
            )
        """)

        db.execSQL("""
            CREATE TABLE IF NOT EXISTS price_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                price REAL NOT NULL,
                source TEXT,
                timestamp INTEGER DEFAULT (strftime('%s','now'))
            )
        """)

        db.execSQL("""
            CREATE TABLE IF NOT EXISTS daily_prices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT UNIQUE,
                open REAL, high REAL, low REAL, close REAL,
                volume REAL,
                source TEXT
            )
        """)

        db.execSQL("""
            CREATE TABLE IF NOT EXISTS onchain_txs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                txid TEXT, direction TEXT, amount_sats INTEGER,
                address TEXT, btc_price REAL, status TEXT DEFAULT 'pending',
                confirmations INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s','now'))
            )
        """)

        createPendingStabilitySendTable(db)

        db.execSQL("CREATE INDEX IF NOT EXISTS idx_price_history_ts ON price_history(timestamp)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_payments_created ON payments(created_at)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_trades_created ON trades(created_at)")
        createTradeIndexes(db)
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_onchain_txs_created ON onchain_txs(created_at)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) {
            db.execSQL("ALTER TABLE channels ADD COLUMN receiver_sats INTEGER NOT NULL DEFAULT 0")
            db.execSQL("ALTER TABLE channels ADD COLUMN latest_price REAL NOT NULL DEFAULT 0.0")
        }
        if (oldVersion < 3) {
            db.execSQL("ALTER TABLE channels ADD COLUMN sync_version INTEGER NOT NULL DEFAULT 0")
            listOf(
                "user_channel_id TEXT",
                "trade_id TEXT",
                "request_hash TEXT",
                "request_payload TEXT",
                "trade_payment_id TEXT",
                "old_expected_usd REAL",
                "new_expected_usd REAL",
                "new_backing_sats INTEGER",
                "quote_price REAL",
                "fee_msat INTEGER NOT NULL DEFAULT 0",
                "expires_at INTEGER",
                "outcome TEXT",
                "reason_code TEXT",
                "uncertainty_reason TEXT",
                "resolved_at INTEGER"
            ).forEach { column -> db.execSQL("ALTER TABLE trades ADD COLUMN $column") }
            createTradeIndexes(db)
        }
    }

    override fun onOpen(db: SQLiteDatabase) {
        super.onOpen(db)
        // IF NOT EXISTS so either process (main app or background service) can create it,
        // including on databases created before this table existed.
        createPendingStabilitySendTable(db)
        createTradeIndexes(db)
    }

    private fun createPendingStabilitySendTable(db: SQLiteDatabase) {
        db.execSQL("""
            CREATE TABLE IF NOT EXISTS pending_stability_send (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                payment_id TEXT NOT NULL,
                amount_msat INTEGER NOT NULL,
                price REAL NOT NULL,
                created_at INTEGER NOT NULL
            )
        """)
    }

    private fun createTradeIndexes(db: SQLiteDatabase) {
        db.execSQL("CREATE UNIQUE INDEX IF NOT EXISTS idx_trades_trade_id_unique ON trades(trade_id) WHERE trade_id IS NOT NULL")
        db.execSQL("CREATE UNIQUE INDEX IF NOT EXISTS idx_trades_request_hash_unique ON trades(request_hash) WHERE request_hash IS NOT NULL")
        db.execSQL("CREATE UNIQUE INDEX IF NOT EXISTS idx_trades_payment_id_unique ON trades(trade_payment_id) WHERE trade_payment_id IS NOT NULL")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_trades_unresolved_channel ON trades(channel_id, status)")
    }

    // --- Channels ---

    fun saveChannel(channelId: String, userChannelId: String, expectedUSD: Double, backingSats: Long, note: String?, receiverSats: Long = 0, latestPrice: Double = 0.0) {
        val db = writableDatabase
        val now = System.currentTimeMillis() / 1000
        val cv = ContentValues().apply {
            put("channel_id", channelId)
            put("user_channel_id", userChannelId)
            put("expected_usd", expectedUSD)
            put("stable_sats", backingSats)
            put("note", note)
            put("receiver_sats", receiverSats)
            put("latest_price", latestPrice)
            put("updated_at", now)
        }
        val updated = db.update("channels", cv, "user_channel_id = ?", arrayOf(userChannelId))
        if (updated == 0) {
            cv.put("created_at", now)
            db.insertWithOnConflict("channels", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
        }
    }

    /**
     * Persist channel metadata without touching stable_sats.
     *
     * Incoming stability payments update stable_sats transactionally. Keeping that column out of
     * this follow-up write prevents stale in-memory state from undoing a concurrent DB increment.
     */
    fun saveChannelPreservingBacking(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        note: String?,
        receiverSats: Long = 0,
        latestPrice: Double = 0.0
    ) {
        val cv = ContentValues().apply {
            put("channel_id", channelId)
            put("expected_usd", expectedUSD)
            put("note", note)
            put("receiver_sats", receiverSats)
            put("latest_price", latestPrice)
            put("updated_at", System.currentTimeMillis() / 1000)
        }
        val rows = writableDatabase.update(
            "channels",
            cv,
            "user_channel_id = ?",
            arrayOf(userChannelId)
        )
        if (rows != 1) {
            throw IllegalStateException(
                "channel metadata UPDATE affected $rows rows for user_channel_id=$userChannelId"
            )
        }
    }

    fun loadChannel(userChannelId: String): ChannelRecord? {
        val db = readableDatabase
        val cursor = db.rawQuery(
            "SELECT channel_id, user_channel_id, expected_usd, note, stable_sats, receiver_sats, latest_price, sync_version FROM channels WHERE user_channel_id = ?",
            arrayOf(userChannelId)
        )
        return cursor.use {
            if (it.moveToFirst()) {
                ChannelRecord(
                    channelId = it.getString(0),
                    userChannelId = it.getString(1),
                    expectedUSD = it.getDouble(2),
                    note = it.getStringOrNull(3),
                    backingSats = it.getLong(4),
                    receiverSats = it.getLong(5),
                    latestPrice = it.getDouble(6),
                    syncVersion = it.getLong(7)
                )
            } else null
        }
    }

    fun deleteChannel(userChannelId: String) {
        writableDatabase.delete("channels", "user_channel_id = ?", arrayOf(userChannelId))
    }

    /** Persisted second source of truth for the LSP-switch gate: true if any channel row exists. */
    fun hasAnyChannel(): Boolean {
        val cursor = readableDatabase.rawQuery("SELECT 1 FROM channels LIMIT 1", null)
        return cursor.use { it.moveToFirst() }
    }

    // --- Trades ---

    fun recordTrade(
        channelId: String, action: String, amountUSD: Double, amountBTC: Double,
        btcPrice: Double, feeUSD: Double, paymentId: String?, status: String = "pending"
    ): Long {
        val cv = ContentValues().apply {
            put("channel_id", channelId)
            put("action", action)
            put("amount_usd", amountUSD)
            put("amount_btc", amountBTC)
            put("btc_price", btcPrice)
            put("fee_usd", feeUSD)
            put("payment_id", paymentId)
            put("status", status)
        }
        return writableDatabase.insert("trades", null, cv)
    }

    fun getRecentTrades(limit: Int = 50): List<TradeRecord> {
        val cursor = readableDatabase.rawQuery(
            "SELECT id, channel_id, action, amount_usd, amount_btc, btc_price, fee_usd, payment_id, status, created_at FROM trades ORDER BY created_at DESC LIMIT ?",
            arrayOf(limit.toString())
        )
        return cursor.use { c ->
            val list = mutableListOf<TradeRecord>()
            while (c.moveToNext()) {
                list.add(TradeRecord(
                    id = c.getLong(0), channelId = c.getString(1), action = c.getString(2),
                    amountUSD = c.getDouble(3), amountBTC = c.getDouble(4), btcPrice = c.getDouble(5),
                    feeUSD = c.getDouble(6), paymentId = c.getStringOrNull(7),
                    status = c.getString(8), createdAt = c.getLong(9)
                ))
            }
            list
        }
    }

    fun updateTradeStatus(tradeId: Long, status: String) {
        val cv = ContentValues().apply { put("status", status) }
        writableDatabase.update("trades", cv, "id = ?", arrayOf(tradeId.toString()))
    }

    fun recordPreparedTrade(trade: PreparedTrade): Long {
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val unresolved = db.rawQuery(
                "SELECT 1 FROM trades WHERE channel_id = ? AND status IN ('prepared','sent','fee_paid','uncertain') LIMIT 1",
                arrayOf(trade.channelId)
            ).use { it.moveToFirst() }
            if (unresolved) throw IllegalStateException("A previous trade is still awaiting its signed result")
            val cv = ContentValues().apply {
                put("channel_id", trade.channelId)
                put("user_channel_id", trade.userChannelId)
                put("action", trade.action)
                put("amount_usd", trade.amountUsd)
                put("amount_btc", trade.amountBtc)
                put("btc_price", trade.quotePrice)
                put("fee_usd", trade.feeUsd)
                put("status", "prepared")
                put("trade_id", trade.tradeId)
                put("request_hash", trade.requestHash)
                put("request_payload", trade.requestPayload)
                put("old_expected_usd", trade.oldExpectedUsd)
                put("new_expected_usd", trade.newExpectedUsd)
                put("new_backing_sats", trade.newBackingSats)
                put("quote_price", trade.quotePrice)
                put("fee_msat", trade.feeMsat)
                put("expires_at", trade.expiresAt)
                put("created_at", trade.createdAt)
            }
            val id = db.insertOrThrow("trades", null, cv)
            db.execSQL("COMMIT")
            return id
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        }
    }

    fun attachTradePaymentId(tradeDbId: Long, paymentId: String): Boolean {
        if (!TradeProtocol.isCanonicalIdentifier(paymentId)) return false
        val cv = ContentValues().apply {
            put("payment_id", paymentId)
            put("trade_payment_id", paymentId)
            put("status", "sent")
        }
        return writableDatabase.update(
            "trades", cv, "id = ? AND status = 'prepared'", arrayOf(tradeDbId.toString())
        ) == 1
    }

    fun markTradeFeePaid(paymentId: String): Boolean {
        val cv = ContentValues().apply { put("status", "fee_paid") }
        return writableDatabase.update(
            "trades", cv,
            "trade_payment_id = ? AND status IN ('prepared','sent','uncertain')",
            arrayOf(paymentId)
        ) == 1
    }

    fun markKnownTradeFeePaid(tradeDbId: Long, paymentId: String): Boolean {
        if (!TradeProtocol.isCanonicalIdentifier(paymentId)) return false
        val cv = ContentValues().apply {
            put("payment_id", paymentId)
            put("trade_payment_id", paymentId)
            put("status", "fee_paid")
        }
        return writableDatabase.update(
            "trades", cv,
            "id = ? AND status IN ('prepared','sent','uncertain') AND (trade_payment_id IS NULL OR trade_payment_id = ?)",
            arrayOf(tradeDbId.toString(), paymentId)
        ) == 1
    }

    fun tradePaymentExists(paymentId: String): Boolean = readableDatabase.rawQuery(
        "SELECT 1 FROM trades WHERE trade_payment_id = ? LIMIT 1",
        arrayOf(paymentId)
    ).use { it.moveToFirst() }

    fun tradeIsUnresolved(tradeDbId: Long): Boolean = readableDatabase.rawQuery(
        "SELECT 1 FROM trades WHERE id = ? AND status IN ('prepared','sent','fee_paid','uncertain') LIMIT 1",
        arrayOf(tradeDbId.toString())
    ).use { it.moveToFirst() }

    fun hasUnattachedPreparedTrade(): Boolean = readableDatabase.rawQuery(
        "SELECT 1 FROM trades WHERE trade_payment_id IS NULL AND status = 'prepared' LIMIT 1",
        null
    ).use { it.moveToFirst() }

    fun adoptUnattachedPreparedTrade(paymentId: String, amountMsat: Long): PendingTradePayment? {
        if (!TradeProtocol.isCanonicalIdentifier(paymentId) || amountMsat < 0L) return null
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val cutoff = System.currentTimeMillis() / 1000L - TradeProtocol.RESPONSE_RETRY_WINDOW_SECS
            val rows = mutableListOf<Array<Any>>()
            db.rawQuery(
                """
                SELECT id, new_expected_usd, quote_price, action
                FROM trades
                WHERE trade_payment_id IS NULL AND status = 'prepared'
                  AND fee_msat = ? AND created_at >= ?
                ORDER BY id DESC LIMIT 2
                """.trimIndent(),
                arrayOf(amountMsat.toString(), cutoff.toString())
            ).use { cursor ->
                while (cursor.moveToNext()) {
                    rows.add(arrayOf(
                        cursor.getLong(0), cursor.getDouble(1),
                        cursor.getDouble(2), cursor.getString(3)
                    ))
                }
            }
            if (rows.size != 1) {
                db.execSQL("ROLLBACK")
                return null
            }
            val row = rows.single()
            val tradeDbId = row[0] as Long
            val cv = ContentValues().apply {
                put("payment_id", paymentId)
                put("trade_payment_id", paymentId)
                put("status", "fee_paid")
            }
            if (db.update(
                    "trades", cv,
                    "id = ? AND trade_payment_id IS NULL AND status = 'prepared'",
                    arrayOf(tradeDbId.toString())
                ) != 1
            ) {
                db.execSQL("ROLLBACK")
                return null
            }
            db.execSQL("COMMIT")
            return PendingTradePayment(
                newExpectedUSD = row[1] as Double,
                price = row[2] as Double,
                tradeDbId = tradeDbId,
                action = row[3] as String,
                status = "fee_paid"
            )
        } catch (error: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw error
        }
    }

    fun failUnattachedPreparedTrade(paymentId: String, amountMsat: Long): PendingTradePayment? {
        if (!TradeProtocol.isCanonicalIdentifier(paymentId) || amountMsat < 0L) return null
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val cutoff = System.currentTimeMillis() / 1000L - TradeProtocol.RESPONSE_RETRY_WINDOW_SECS
            val rows = mutableListOf<Array<Any>>()
            db.rawQuery(
                """
                SELECT id, new_expected_usd, quote_price, action
                FROM trades
                WHERE trade_payment_id IS NULL AND status = 'prepared'
                  AND fee_msat = ? AND created_at >= ?
                ORDER BY id DESC LIMIT 2
                """.trimIndent(),
                arrayOf(amountMsat.toString(), cutoff.toString())
            ).use { cursor ->
                while (cursor.moveToNext()) {
                    rows.add(arrayOf(
                        cursor.getLong(0), cursor.getDouble(1),
                        cursor.getDouble(2), cursor.getString(3)
                    ))
                }
            }
            if (rows.size != 1) {
                db.execSQL("ROLLBACK")
                return null
            }
            val row = rows.single()
            val tradeDbId = row[0] as Long
            val cv = ContentValues().apply {
                put("payment_id", paymentId)
                put("trade_payment_id", paymentId)
                put("status", "send_failed")
                put("outcome", "send_failed")
                put("resolved_at", System.currentTimeMillis() / 1000L)
            }
            if (db.update(
                    "trades", cv,
                    "id = ? AND trade_payment_id IS NULL AND status = 'prepared'",
                    arrayOf(tradeDbId.toString())
                ) != 1
            ) {
                db.execSQL("ROLLBACK")
                return null
            }
            db.execSQL("COMMIT")
            return PendingTradePayment(
                newExpectedUSD = row[1] as Double,
                price = row[2] as Double,
                tradeDbId = tradeDbId,
                action = row[3] as String,
                status = "send_failed"
            )
        } catch (error: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw error
        }
    }

    fun markTradeSendFailed(tradeDbId: Long): Boolean {
        val cv = ContentValues().apply {
            put("status", "send_failed")
            put("outcome", "send_failed")
            put("resolved_at", System.currentTimeMillis() / 1000L)
        }
        return writableDatabase.update(
            "trades", cv, "id = ? AND status IN ('prepared','sent','uncertain')",
            arrayOf(tradeDbId.toString())
        ) == 1
    }

    /** Terminal outcome for a trade's fee payment id, straight from SQLite — the source
     *  of truth that BOTH the foreground handler and the background service write. The
     *  in-memory outcome map alone misses results committed while the app was backgrounded
     *  or before a restart. Returns (accepted, reason_code) or null while unresolved. */
    fun terminalTradeOutcome(paymentId: String): Pair<Boolean, String?>? {
        val cursor = readableDatabase.rawQuery(
            "SELECT status, reason_code FROM trades WHERE trade_payment_id = ? AND status IN ('accepted','rejected') ORDER BY id DESC LIMIT 1",
            arrayOf(paymentId)
        )
        return cursor.use { c ->
            if (c.moveToFirst()) Pair(c.getString(0) == "accepted", c.getString(1)) else null
        }
    }

    fun unresolvedTradePayments(): Map<String, PendingTradePayment> {
        val cursor = readableDatabase.rawQuery(
            """
            SELECT trade_payment_id, new_expected_usd, quote_price, id, action, status
            FROM trades
            WHERE trade_payment_id IS NOT NULL
              AND status IN ('sent','fee_paid','uncertain')
            ORDER BY id
            """.trimIndent(), null
        )
        return cursor.use { c ->
            buildMap {
                while (c.moveToNext()) {
                    put(c.getString(0), PendingTradePayment(
                        newExpectedUSD = c.getDouble(1),
                        price = c.getDouble(2),
                        tradeDbId = c.getLong(3),
                        action = c.getString(4),
                        status = c.getString(5)
                    ))
                }
            }
        }
    }

    fun markExpiredTradesUncertain(now: Long = System.currentTimeMillis() / 1000L): Int {
        val cv = ContentValues().apply {
            put("status", "uncertain")
            put("uncertainty_reason", "no_response")
        }
        return writableDatabase.update(
            "trades", cv,
            "expires_at IS NOT NULL AND expires_at <= ? AND status IN ('prepared','sent','fee_paid')",
            arrayOf(now.toString())
        )
    }

    fun markTradeResponseNotCommittable(message: TradeControlMessage): Boolean {
        val channelId: String
        val correlation: TradeCorrelation
        when (message) {
            is TradeControlMessage.Sync -> {
                channelId = message.channelId
                correlation = message.correlation ?: return false
            }
            is TradeControlMessage.Rejected -> {
                channelId = message.channelId
                correlation = message.correlation
            }
        }
        val cv = ContentValues().apply {
            put("status", "uncertain")
            put("uncertainty_reason", "response_not_committable")
        }
        return writableDatabase.update(
            "trades", cv,
            """
            channel_id = ? AND trade_id = ? AND request_hash = ?
              AND (trade_payment_id = ? OR trade_payment_id IS NULL)
              AND status IN ('prepared','sent','fee_paid','uncertain')
            """.trimIndent(),
            arrayOf(
                channelId, correlation.tradeId, correlation.requestHash,
                correlation.tradePaymentId
            )
        ) == 1
    }

    fun applyCorrelatedTradeAcceptance(sync: TradeControlMessage.Sync): TradeControlApplyResult {
        val correlation = sync.correlation ?: return TradeControlApplyResult(TradeControlApplyStatus.INVALID)
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val trade = db.rawQuery(
                """
                SELECT id, channel_id, user_channel_id, trade_payment_id, new_expected_usd,
                       new_backing_sats, status, action
                FROM trades
                WHERE trade_id = ? AND request_hash = ?
                  AND (trade_payment_id = ? OR trade_payment_id IS NULL)
                LIMIT 1
                """.trimIndent(),
                arrayOf(correlation.tradeId, correlation.requestHash, correlation.tradePaymentId)
            ).use { c ->
                if (!c.moveToFirst()) null else arrayOf<Any?>(
                    c.getLong(0), c.getString(1), c.getString(2), c.getStringOrNull(3),
                    c.getDouble(4), c.getLong(5), c.getString(6), c.getString(7)
                )
            } ?: return rollbackResult(db, TradeControlApplyStatus.INVALID)
            val tradeId = trade[0] as Long
            val tradeChannelId = trade[1] as String
            val tradeUserChannelId = trade[2] as String
            val storedPaymentId = trade[3] as String?
            val storedExpected = trade[4] as Double
            val storedBacking = trade[5] as Long
            val status = trade[6] as String
            val action = trade[7] as String
            if (tradeChannelId != sync.channelId || tradeUserChannelId != sync.userChannelId ||
                storedPaymentId != null && storedPaymentId != correlation.tradePaymentId ||
                kotlin.math.abs(storedExpected - sync.expectedUsd) > 0.000000001
            ) return rollbackResult(db, TradeControlApplyStatus.INVALID)
            if (status == "accepted") {
                db.execSQL("ROLLBACK")
                return TradeControlApplyResult(
                    TradeControlApplyStatus.DUPLICATE, storedBacking, sync.backingSats,
                    correlation.tradePaymentId, action
                )
            }
            if (status == "rejected" || status == "send_failed") {
                return rollbackResult(db, TradeControlApplyStatus.INVALID)
            }
            val channel = db.rawQuery(
                "SELECT channel_id, receiver_sats, sync_version, stable_sats FROM channels WHERE user_channel_id = ?",
                arrayOf(sync.userChannelId)
            ).use { c ->
                if (!c.moveToFirst()) null else arrayOf<Any>(
                    c.getString(0), c.getLong(1), c.getLong(2), c.getLong(3)
                )
            } ?: return rollbackResult(db, TradeControlApplyStatus.RETRY)
            if (channel[0] as String != sync.channelId) {
                return rollbackResult(db, TradeControlApplyStatus.INVALID)
            }
            val receiverSats = channel[1] as Long
            val currentVersion = channel[2] as Long
            val currentBacking = channel[3] as Long
            val allocationApplied = sync.syncVersion > currentVersion
            if (allocationApplied) {
                if (storedBacking < 0L || storedBacking > receiverSats) {
                    return rollbackResult(db, TradeControlApplyStatus.RETRY)
                }
                val channelValues = ContentValues().apply {
                    put("expected_usd", sync.expectedUsd)
                    put("stable_sats", storedBacking)
                    put("sync_version", sync.syncVersion)
                    put("updated_at", System.currentTimeMillis() / 1000L)
                }
                if (db.update(
                        "channels", channelValues,
                        "user_channel_id = ? AND channel_id = ? AND sync_version < ?",
                        arrayOf(sync.userChannelId, sync.channelId, sync.syncVersion.toString())
                    ) != 1
                ) return rollbackResult(db, TradeControlApplyStatus.RETRY)
            }
            val tradeValues = ContentValues().apply {
                put("payment_id", correlation.tradePaymentId)
                put("trade_payment_id", correlation.tradePaymentId)
                put("status", "accepted")
                put("outcome", "accepted")
                put("resolved_at", System.currentTimeMillis() / 1000L)
                putNull("uncertainty_reason")
            }
            if (db.update("trades", tradeValues, "id = ?", arrayOf(tradeId.toString())) != 1) {
                return rollbackResult(db, TradeControlApplyStatus.RETRY)
            }
            db.execSQL("COMMIT")
            return TradeControlApplyResult(
                TradeControlApplyStatus.APPLIED,
                if (allocationApplied) storedBacking else currentBacking,
                sync.backingSats,
                correlation.tradePaymentId, action, allocationApplied
            )
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        }
    }

    fun applyTradeRejection(rejection: TradeControlMessage.Rejected): TradeControlApplyResult {
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val row = db.rawQuery(
                """
                SELECT id, channel_id, trade_payment_id, status, action
                FROM trades WHERE trade_id = ? AND request_hash = ?
                  AND (trade_payment_id = ? OR trade_payment_id IS NULL) LIMIT 1
                """.trimIndent(),
                arrayOf(
                    rejection.correlation.tradeId,
                    rejection.correlation.requestHash,
                    rejection.correlation.tradePaymentId
                )
            ).use { c ->
                if (!c.moveToFirst()) null else arrayOf<Any?>(
                    c.getLong(0), c.getString(1), c.getStringOrNull(2), c.getString(3), c.getString(4)
                )
            } ?: return rollbackResult(db, TradeControlApplyStatus.INVALID)
            val id = row[0] as Long
            val channelId = row[1] as String
            val storedPayment = row[2] as String?
            val status = row[3] as String
            val action = row[4] as String
            if (channelId != rejection.channelId ||
                storedPayment != null && storedPayment != rejection.correlation.tradePaymentId
            ) return rollbackResult(db, TradeControlApplyStatus.INVALID)
            if (status == "rejected") {
                db.execSQL("ROLLBACK")
                return TradeControlApplyResult(
                    TradeControlApplyStatus.DUPLICATE,
                    paymentId = rejection.correlation.tradePaymentId,
                    action = action
                )
            }
            if (status == "accepted" || status == "send_failed") {
                return rollbackResult(db, TradeControlApplyStatus.INVALID)
            }
            val cv = ContentValues().apply {
                put("payment_id", rejection.correlation.tradePaymentId)
                put("trade_payment_id", rejection.correlation.tradePaymentId)
                put("status", "rejected")
                put("outcome", "rejected")
                put("reason_code", rejection.reasonCode)
                put("resolved_at", rejection.decidedAt)
                putNull("uncertainty_reason")
            }
            if (db.update("trades", cv, "id = ?", arrayOf(id.toString())) != 1) {
                return rollbackResult(db, TradeControlApplyStatus.RETRY)
            }
            db.execSQL("COMMIT")
            return TradeControlApplyResult(
                TradeControlApplyStatus.APPLIED,
                paymentId = rejection.correlation.tradePaymentId,
                action = action
            )
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        }
    }

    fun applyUncorrelatedSyncIfNewer(
        sync: TradeControlMessage.Sync,
        trustedPrice: Double
    ): TradeControlApplyResult {
        if (sync.correlation != null || !trustedPrice.isFinite() || trustedPrice <= 0.0) {
            return TradeControlApplyResult(TradeControlApplyStatus.INVALID)
        }
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val row = db.rawQuery(
                """
                SELECT channel_id, expected_usd, stable_sats, receiver_sats, sync_version
                FROM channels WHERE user_channel_id = ?
                """.trimIndent(), arrayOf(sync.userChannelId)
            ).use { c ->
                if (!c.moveToFirst()) null else arrayOf<Any>(
                    c.getString(0), c.getDouble(1), c.getLong(2), c.getLong(3), c.getLong(4)
                )
            } ?: return rollbackResult(db, TradeControlApplyStatus.RETRY)
            if (row[0] as String != sync.channelId) return rollbackResult(db, TradeControlApplyStatus.INVALID)
            val currentVersion = row[4] as Long
            if (sync.syncVersion <= currentVersion) {
                db.execSQL("ROLLBACK")
                return TradeControlApplyResult(TradeControlApplyStatus.DUPLICATE)
            }
            val currentExpected = row[1] as Double
            val currentBacking = row[2] as Long
            val receiverSats = row[3] as Long
            val localBacking = if (sync.expectedUsd == 0.0) 0L else if (
                currentBacking > 0L && sync.expectedUsd == currentExpected
            ) {
                currentBacking.coerceAtMost(receiverSats)
            } else {
                TradeProtocol.tradeBackingAfterDelta(
                    receiverSats, currentBacking, currentExpected, sync.expectedUsd, trustedPrice
                ) ?: return rollbackResult(db, TradeControlApplyStatus.RETRY)
            }
            val cv = ContentValues().apply {
                put("expected_usd", sync.expectedUsd)
                put("stable_sats", localBacking)
                put("sync_version", sync.syncVersion)
                put("latest_price", trustedPrice)
                put("updated_at", System.currentTimeMillis() / 1000L)
            }
            if (db.update(
                    "channels", cv,
                    "user_channel_id = ? AND channel_id = ? AND sync_version < ?",
                    arrayOf(sync.userChannelId, sync.channelId, sync.syncVersion.toString())
                ) != 1
            ) return rollbackResult(db, TradeControlApplyStatus.RETRY)
            db.execSQL("COMMIT")
            return TradeControlApplyResult(
                TradeControlApplyStatus.APPLIED, localBacking, sync.backingSats
            )
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        }
    }

    private fun rollbackResult(
        db: SQLiteDatabase,
        status: TradeControlApplyStatus
    ): TradeControlApplyResult {
        try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
        return TradeControlApplyResult(status)
    }

    // --- Payments ---

    fun recordPayment(
        paymentId: String?, paymentType: String, direction: String, amountMsat: Long,
        amountUSD: Double? = null, btcPrice: Double? = null, counterparty: String? = null,
        status: String = "completed", txid: String? = null, address: String? = null
    ): Long {
        // Dedup: skip if payment_id already exists
        if (!paymentId.isNullOrEmpty()) {
            val cursor = readableDatabase.rawQuery(
                "SELECT id FROM payments WHERE payment_id = ?", arrayOf(paymentId)
            )
            val exists = cursor.use { it.moveToFirst() }
            if (exists) return -1
        }

        val cv = ContentValues().apply {
            put("payment_id", paymentId)
            put("payment_type", paymentType)
            put("direction", direction)
            put("amount_msat", amountMsat)
            put("amount_usd", amountUSD)
            put("btc_price", btcPrice)
            put("counterparty", counterparty)
            put("status", status)
            put("txid", txid)
            put("address", address)
        }
        return writableDatabase.insert("payments", null, cv)
    }

    /** Insert a payment and atomically update channel backing sats in one SQLite transaction.
     *  Returns whether the payment was new and the authoritative backing value, when applicable. */
    fun recordPaymentAndMaybeUpdateBacking(
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: Long,
        amountUSD: Double? = null,
        btcPrice: Double? = null,
        counterparty: String? = null,
        userChannelId: String? = null,
        backingDeltaSats: Long? = null
    ): PaymentPersistenceResult {
        val db = writableDatabase
        // BEGIN IMMEDIATE acquires the write lock before the dedup SELECT, preventing
        // a TOCTOU race where two processes both read "not exists" then both INSERT.
        db.execSQL("BEGIN IMMEDIATE")
        try {
            // Dedup check inside the write lock
            if (!paymentId.isNullOrEmpty()) {
                val cursor = db.rawQuery("SELECT id FROM payments WHERE payment_id = ?", arrayOf(paymentId))
                val exists = cursor.use { it.moveToFirst() }
                if (exists) {
                    val backing = if (backingDeltaSats != null) {
                        val ucid = userChannelId
                            ?: throw IllegalStateException("userChannelId required for backing update")
                        readBackingSats(db, ucid)
                            ?: throw MissingChannelRowException(ucid)
                    } else {
                        null
                    }
                    db.execSQL("ROLLBACK")
                    return PaymentPersistenceResult(false, backing)
                }
            }
            val cv = ContentValues().apply {
                put("payment_id", paymentId)
                put("payment_type", paymentType)
                put("direction", direction)
                put("amount_msat", amountMsat)
                put("amount_usd", amountUSD)
                put("btc_price", btcPrice)
                put("counterparty", counterparty)
                put("status", "completed")
            }
            db.insertOrThrow("payments", null, cv)
            var resultingBacking: Long? = null
            if (backingDeltaSats != null) {
                val ucid = userChannelId
                    ?: throw IllegalStateException("userChannelId required for backing update")
                val current = readBackingSats(db, ucid)
                    ?: throw MissingChannelRowException(ucid)
                // Clamp instead of refusing: this runs after the payment already settled, so the
                // sats truly moved — a floor of 0 keeps the ledger recordable instead of wedging.
                val newBacking = maxOf(0L, current + backingDeltaSats)
                if (current + backingDeltaSats < 0) {
                    AuditService.log("BACKING_CLAMPED", mapOf(
                        "user_channel_id" to ucid,
                        "current_backing_sats" to current,
                        "delta_sats" to backingDeltaSats,
                        "clamped_to" to newBacking
                    ))
                }
                val stmt = db.compileStatement(
                    "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s','now') WHERE user_channel_id = ?"
                )
                stmt.bindLong(1, newBacking)
                stmt.bindString(2, ucid)
                val rows = stmt.executeUpdateDelete()
                if (rows != 1) {
                    throw IllegalStateException(
                        "backing UPDATE affected $rows rows for user_channel_id=$ucid"
                    )
                }
                resultingBacking = newBacking
            }
            db.execSQL("COMMIT")
            return PaymentPersistenceResult(true, resultingBacking)
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        }
    }

    private fun readBackingSats(db: SQLiteDatabase, userChannelId: String): Long? {
        val cursor = db.rawQuery(
            "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
            arrayOf(userChannelId)
        )
        return cursor.use { if (it.moveToFirst()) it.getLong(0) else null }
    }

    // --- Pending outgoing stability send marker (single row, id = 1) ---

    /** Atomically claim the right to send an outgoing stability payment.
     *  Returns false when a marker already exists (another sender owns the send).
     *  BEGIN IMMEDIATE makes the check-and-insert a single atomic step across processes. */
    fun claimPendingSend(amountMsat: Long, price: Double): Boolean {
        val db = writableDatabase
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val cursor = db.rawQuery("SELECT id FROM pending_stability_send WHERE id = 1", null)
            val exists = cursor.use { it.moveToFirst() }
            if (exists) {
                db.execSQL("ROLLBACK")
                return false
            }
            db.execSQL(
                "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)",
                arrayOf<Any?>(amountMsat, price, System.currentTimeMillis() / 1000)
            )
            db.execSQL("COMMIT")
            return true
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        }
    }

    fun setPendingSendPaymentId(paymentId: String) {
        writableDatabase.execSQL(
            "UPDATE pending_stability_send SET payment_id = ? WHERE id = 1",
            arrayOf(paymentId)
        )
    }

    fun loadPendingSend(): PendingStabilitySend? {
        val cursor = readableDatabase.rawQuery(
            "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1",
            null
        )
        return cursor.use {
            if (it.moveToFirst()) {
                PendingStabilitySend(
                    paymentId = it.getString(0),
                    amountMsat = it.getLong(1),
                    price = it.getDouble(2),
                    createdAt = it.getLong(3)
                )
            } else null
        }
    }

    fun clearPendingSend() {
        writableDatabase.execSQL("DELETE FROM pending_stability_send WHERE id = 1")
    }

    fun getRecentPayments(limit: Int = 50): List<PaymentRecord> {
        val cursor = readableDatabase.rawQuery(
            "SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, created_at, fee_msat, txid, address, confirmations FROM payments WHERE NOT (payment_type = 'lightning' AND amount_msat < 1000) ORDER BY created_at DESC LIMIT ?",
            arrayOf(limit.toString())
        )
        return cursor.use { c ->
            val list = mutableListOf<PaymentRecord>()
            while (c.moveToNext()) {
                list.add(PaymentRecord(
                    id = c.getLong(0), paymentId = c.getStringOrNull(1),
                    paymentType = c.getString(2), direction = c.getString(3),
                    amountMsat = c.getLong(4), amountUSD = c.getDoubleOrNull(5),
                    btcPrice = c.getDoubleOrNull(6), counterparty = c.getStringOrNull(7),
                    status = c.getString(8), createdAt = c.getLong(9),
                    feeMsat = c.getLong(10), txid = c.getStringOrNull(11),
                    address = c.getStringOrNull(12), confirmations = c.getInt(13)
                ))
            }
            list
        }
    }

    fun latestPendingOnchainReceive(): PaymentRecord? {
        val cursor = readableDatabase.rawQuery(
            """
            SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, created_at, fee_msat, txid, address, confirmations
            FROM payments
            WHERE payment_type = 'onchain'
              AND direction = 'received'
              AND status = 'pending'
            ORDER BY created_at DESC
            LIMIT 1
            """.trimIndent(),
            null
        )
        return cursor.use { c ->
            if (!c.moveToFirst()) return@use null
            PaymentRecord(
                id = c.getLong(0), paymentId = c.getStringOrNull(1),
                paymentType = c.getString(2), direction = c.getString(3),
                amountMsat = c.getLong(4), amountUSD = c.getDoubleOrNull(5),
                btcPrice = c.getDoubleOrNull(6), counterparty = c.getStringOrNull(7),
                status = c.getString(8), createdAt = c.getLong(9),
                feeMsat = c.getLong(10), txid = c.getStringOrNull(11),
                address = c.getStringOrNull(12), confirmations = c.getInt(13)
            )
        }
    }

    fun getPaymentsNeedingConfirmation(limit: Int = 50): List<PaymentRecord> {
        val cursor = readableDatabase.rawQuery(
            """
            SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, created_at, fee_msat, txid, address, confirmations
            FROM payments
            WHERE txid IS NOT NULL AND txid != ''
              AND payment_type IN ('onchain', 'channel_close', 'splice_in', 'splice_out')
              AND status != 'failed'
              AND (
                    (payment_type IN ('onchain', 'channel_close') AND confirmations < 6)
                    OR
                    (payment_type IN ('splice_in', 'splice_out') AND confirmations < 1)
                  )
            ORDER BY created_at DESC
            LIMIT ?
            """.trimIndent(),
            arrayOf(limit.toString())
        )
        return cursor.use { c ->
            val list = mutableListOf<PaymentRecord>()
            while (c.moveToNext()) {
                list.add(PaymentRecord(
                    id = c.getLong(0), paymentId = c.getStringOrNull(1),
                    paymentType = c.getString(2), direction = c.getString(3),
                    amountMsat = c.getLong(4), amountUSD = c.getDoubleOrNull(5),
                    btcPrice = c.getDoubleOrNull(6), counterparty = c.getStringOrNull(7),
                    status = c.getString(8), createdAt = c.getLong(9),
                    feeMsat = c.getLong(10), txid = c.getStringOrNull(11),
                    address = c.getStringOrNull(12), confirmations = c.getInt(13)
                ))
            }
            list
        }
    }

    fun updatePaymentConfirmationState(paymentRowId: Long, confirmations: Int, status: String): Boolean {
        val cv = ContentValues().apply {
            put("confirmations", confirmations)
            put("status", status)
        }
        return writableDatabase.update(
            "payments",
            cv,
            "id = ?",
            arrayOf(paymentRowId.toString())
        ) > 0
    }

    fun clearPaymentTxidForRow(paymentRowId: Long): Boolean {
        val cv = ContentValues().apply {
            putNull("txid")
            put("confirmations", 0)
            put("status", "pending")
        }
        return writableDatabase.update(
            "payments",
            cv,
            "id = ?",
            arrayOf(paymentRowId.toString())
        ) > 0
    }

    fun updatePaymentStatus(paymentId: String, status: String, feeMsat: Long = 0) {
        val cv = ContentValues().apply {
            put("status", status)
            if (feeMsat > 0) put("fee_msat", feeMsat)
        }
        writableDatabase.update("payments", cv, "payment_id = ?", arrayOf(paymentId))
    }

    fun isOutgoingStabilityPayment(paymentId: String): Boolean {
        val cursor = readableDatabase.rawQuery(
            "SELECT 1 FROM payments WHERE payment_id = ? AND payment_type = 'stability' AND direction = 'sent' LIMIT 1",
            arrayOf(paymentId)
        )
        return cursor.use { it.moveToFirst() }
    }

    fun updatePaymentTxid(paymentId: String, txid: String) {
        val cv = ContentValues().apply {
            put("txid", txid)
        }
        writableDatabase.update("payments", cv, "payment_id = ?", arrayOf(paymentId))
    }

    private data class PendingPlaceholder(val id: Long, val amountMsat: Long)

    /** The newest txid-less pending receive placeholder for an address — the row the
     *  balance-delta path writes before the txid is known. When `amountMsat` is given, only a
     *  placeholder with exactly that amount matches: several deposits to the same (reused)
     *  address can be pending at once, and amount is what tells their placeholders apart. */
    private fun findPendingPlaceholder(
        db: SQLiteDatabase,
        address: String,
        amountMsat: Long? = null
    ): PendingPlaceholder? {
        val amountFilter = if (amountMsat != null) "AND amount_msat = ? " else ""
        val args = if (amountMsat != null) arrayOf(address, amountMsat.toString()) else arrayOf(address)
        return db.rawQuery(
            "SELECT id, amount_msat FROM payments WHERE payment_type = 'onchain' AND direction = 'received' AND address = ? AND (txid IS NULL OR txid = '') AND status = 'pending' " + amountFilter + "ORDER BY created_at DESC LIMIT 1",
            args
        ).use { c -> if (c.moveToFirst()) PendingPlaceholder(c.getLong(0), c.getLong(1)) else null }
    }

    /** Record a websocket-detected receive unless its txid is already tracked. Check and write
     *  run in one transaction so a concurrent balance-delta detection can't double-insert. If the
     *  balance-delta path already wrote a txid-less placeholder for this address, that row is
     *  adopted (txid + exact websocket amount attached) instead of inserting a second row.
     *  Returns the row id, or -1 when the txid is already on any row. */
    fun recordWebSocketReceive(
        paymentId: String,
        amountMsat: Long,
        amountUSD: Double?,
        btcPrice: Double?,
        txid: String,
        address: String
    ): Long {
        val db = writableDatabase
        db.beginTransaction()
        try {
            val alreadyTracked = db.rawQuery(
                "SELECT 1 FROM payments WHERE txid = ? OR payment_id = ? LIMIT 1",
                arrayOf(txid, paymentId)
            ).use { it.moveToFirst() }
            if (alreadyTracked) {
                db.setTransactionSuccessful()
                return -1L
            }

            // Adopt only on an exact amount match: for a single deposit the balance delta
            // equals the vout sum, so a placeholder with a different amount is a DIFFERENT
            // deposit still awaiting its own txid — adopting it would erase that deposit
            // from history. The amount is part of the lookup (not a post-check on the newest
            // row) so the right placeholder is found even when several are pending.
            val placeholder = findPendingPlaceholder(db, address, amountMsat)

            val rowId: Long
            if (placeholder != null) {
                val cv = ContentValues().apply {
                    put("payment_id", paymentId)
                    put("txid", txid)
                    put("amount_msat", amountMsat)
                    amountUSD?.let { put("amount_usd", it) }
                    btcPrice?.let { put("btc_price", it) }
                }
                db.update("payments", cv, "id = ?", arrayOf(placeholder.id.toString()))
                rowId = placeholder.id
            } else {
                val cv = ContentValues().apply {
                    put("payment_id", paymentId)
                    put("payment_type", "onchain")
                    put("direction", "received")
                    put("amount_msat", amountMsat)
                    put("amount_usd", amountUSD)
                    put("btc_price", btcPrice)
                    put("status", "pending")
                    put("txid", txid)
                    put("address", address)
                }
                rowId = db.insert("payments", null, cv)
            }
            db.setTransactionSuccessful()
            return rowId
        } finally {
            db.endTransaction()
        }
    }

    /** Reconcile an HTTP-resolver-resolved txid against the receive rows, in one transaction.
     *  If a row already carries the txid AND its amount matches the placeholder's, the websocket
     *  recorded this same deposit first — the placeholder is a duplicate, delete it. On an amount
     *  mismatch the placeholder is a different deposit whose txid the resolver couldn't tell
     *  apart (the resolver looks up by address, not per-deposit), so it is left alone rather
     *  than deleted or mislabeled. With no websocket row, attach the txid to the placeholder. */
    fun reconcileResolvedReceiveTxid(txid: String, address: String): Boolean {
        val db = writableDatabase
        db.beginTransaction()
        try {
            val websocketRowAmountMsat = db.rawQuery(
                "SELECT amount_msat FROM payments WHERE txid = ? LIMIT 1",
                arrayOf(txid)
            ).use { c -> if (c.moveToFirst()) c.getLong(0) else null }

            val changed = if (websocketRowAmountMsat != null) {
                // The websocket already recorded this deposit; only the placeholder with the
                // SAME amount is its duplicate — a different-amount placeholder belongs to
                // another deposit and must survive.
                findPendingPlaceholder(db, address, websocketRowAmountMsat)?.let {
                    db.delete("payments", "id = ?", arrayOf(it.id.toString())) > 0
                } ?: false
            } else {
                // No amount to disambiguate by (the resolver returns only a txid), so this
                // attaches to the newest placeholder — with several deposits pending it can
                // pick the wrong one. Making the resolver return per-tx vout sums would fix it.
                findPendingPlaceholder(db, address)?.let {
                    val cv = ContentValues().apply { put("txid", txid) }
                    db.update("payments", cv, "id = ?", arrayOf(it.id.toString())) > 0
                } ?: false
            }
            db.setTransactionSuccessful()
            return changed
        } finally {
            db.endTransaction()
        }
    }

    fun failPaymentByTxid(txid: String) {
        writableDatabase.execSQL(
            "UPDATE payments SET status = 'failed' WHERE txid = ? AND status = 'pending'",
            arrayOf(txid)
        )
    }

    fun getPendingChannelClosePaymentId(): String? {
        val cursor = readableDatabase.rawQuery(
            "SELECT payment_id FROM payments WHERE payment_type = 'channel_close' AND status = 'pending' ORDER BY created_at DESC LIMIT 1",
            null
        )
        return cursor.use { if (it.moveToFirst()) it.getString(0) else null }
    }

    fun getPaymentTxid(paymentId: String): String? {
        val cursor = readableDatabase.rawQuery(
            "SELECT txid FROM payments WHERE payment_id = ?",
            arrayOf(paymentId)
        )
        return cursor.use { if (it.moveToFirst()) it.getString(0) else null }
    }

    fun setPendingSpliceTxid(txid: String) {
        writableDatabase.execSQL(
            "UPDATE payments SET txid = ? WHERE rowid = (SELECT rowid FROM payments WHERE payment_type IN ('splice_in','splice_out') AND status IN ('pending','failed') AND txid IS NULL ORDER BY created_at DESC LIMIT 1)",
            arrayOf(txid)
        )
    }

    /** Stamps a txid onto a splice row stuck at NULL (event lost across a restart) and un-fails
     *  it, using the live channel's funding txid as the restart-proof source of truth. */
    fun recoverStuckSpliceTxid(txid: String): Boolean {
        val stmt = writableDatabase.compileStatement(
            "UPDATE payments SET txid = ?, status = 'pending' WHERE rowid = (SELECT rowid FROM payments WHERE payment_type IN ('splice_in','splice_out') AND txid IS NULL ORDER BY created_at DESC LIMIT 1)"
        )
        stmt.bindString(1, txid)
        return stmt.executeUpdateDelete() > 0
    }

    /** Fails a splice-out row if negotiation never even started (no txid assigned yet). */
    fun failPendingSpliceOutWithoutTxid() {
        writableDatabase.execSQL(
            "UPDATE payments SET status = 'failed' WHERE rowid = (SELECT rowid FROM payments WHERE payment_type = 'splice_out' AND status = 'pending' AND txid IS NULL ORDER BY created_at DESC LIMIT 1)"
        )
    }

    fun completeLatestSplice(txid: String?) {
        if (txid.isNullOrBlank()) {
            writableDatabase.execSQL(
                "UPDATE payments SET status = 'completed' WHERE rowid = (SELECT rowid FROM payments WHERE payment_type IN ('splice_in','splice_out') AND status IN ('pending','failed') ORDER BY created_at DESC LIMIT 1)"
            )
        } else {
            writableDatabase.execSQL(
                "UPDATE payments SET status = 'completed' WHERE payment_type IN ('splice_in','splice_out') AND txid = ? AND status IN ('pending','failed')",
                arrayOf(txid)
            )
        }
    }

    /** Returns true only if a splice row was actually flipped to completed,
     *  so callers can use the result as the "this ChannelReady was a splice" signal. */
    fun completeSplice(txid: String): Boolean {
        val stmt = writableDatabase.compileStatement(
            "UPDATE payments SET status = 'completed', confirmations = 1 WHERE payment_type IN ('splice_in','splice_out') AND txid = ? AND status IN ('pending','failed')"
        )
        stmt.bindString(1, txid)
        return stmt.executeUpdateDelete() > 0
    }

    fun failLatestPendingSplice() {
        writableDatabase.execSQL(
            "UPDATE payments SET status = 'failed' WHERE rowid = (SELECT rowid FROM payments WHERE payment_type IN ('splice_in','splice_out') AND status = 'pending' ORDER BY created_at DESC LIMIT 1)"
        )
    }

    fun getPendingSpliceTxid(): String? {
        val cursor = readableDatabase.rawQuery(
            "SELECT txid FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in','splice_out') AND txid IS NOT NULL ORDER BY created_at DESC LIMIT 1",
            null
        )
        return cursor.use { if (it.moveToFirst()) it.getString(0) else null }
    }

    fun hasPendingSplice(): Boolean {
        // If the app died before SpliceNegotiated delivered a txid, there is no
        // durable in-flight splice to wait for. Let that pre-negotiation lock heal.
        // Keep with-txid rows pending: confirmation can outlive the app process,
        // and the splice confirmation monitor completes them after 1 conf.
        val noTxidCutoff = System.currentTimeMillis() / 1000 - 600
        writableDatabase.execSQL(
            "UPDATE payments SET status = 'failed' WHERE status = 'pending' AND payment_type IN ('splice_in','splice_out') AND txid IS NULL AND created_at < ?",
            arrayOf(noTxidCutoff)
        )
        val cursor = readableDatabase.rawQuery(
            "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in','splice_out') LIMIT 1",
            null
        )
        return cursor.use { it.moveToFirst() }
    }

    // --- Prices ---

    fun recordPrice(price: Double, source: String?) {
        val cv = ContentValues().apply {
            put("price", price)
            put("source", source)
        }
        writableDatabase.insert("price_history", null, cv)
    }

    fun getPriceHistory(hours: Int = 24): List<PriceRecord> {
        val cutoff = System.currentTimeMillis() / 1000 - hours * 3600
        val cursor = readableDatabase.rawQuery(
            "SELECT id, price, source, timestamp FROM price_history WHERE timestamp >= ? ORDER BY timestamp ASC",
            arrayOf(cutoff.toString())
        )
        return cursor.use { c ->
            val list = mutableListOf<PriceRecord>()
            while (c.moveToNext()) {
                list.add(PriceRecord(
                    id = c.getLong(0), price = c.getDouble(1),
                    source = c.getStringOrNull(2), timestamp = c.getLong(3)
                ))
            }
            list
        }
    }

    fun getDailyPrices(days: Int = 365): List<DailyPriceRecord> {
        val cursor = readableDatabase.rawQuery(
            "SELECT date, open, high, low, close, volume FROM daily_prices ORDER BY date DESC LIMIT ?",
            arrayOf(days.toString())
        )
        return cursor.use { c ->
            val list = mutableListOf<DailyPriceRecord>()
            while (c.moveToNext()) {
                list.add(DailyPriceRecord(
                    date = c.getString(0), open = c.getDouble(1), high = c.getDouble(2),
                    low = c.getDouble(3), close = c.getDouble(4),
                    volume = c.getDoubleOrNull(5)
                ))
            }
            list
        }
    }

    fun seedHistoricalPrices() {
        val db = writableDatabase
        // Check if already seeded
        val cursor = db.rawQuery("SELECT COUNT(*) FROM daily_prices", null)
        val count = cursor.use { if (it.moveToFirst()) it.getInt(0) else 0 }
        if (count >= 100) return // already seeded

        db.beginTransaction()
        try {
            val stmt = db.compileStatement(
                "INSERT OR IGNORE INTO daily_prices (date, open, high, low, close, source) VALUES (?, ?, ?, ?, ?, 'seed')"
            )
            for (p in HistoricalPrices.seedPrices) {
                stmt.clearBindings()
                stmt.bindString(1, p.date)
                stmt.bindDouble(2, p.open)
                stmt.bindDouble(3, p.high)
                stmt.bindDouble(4, p.low)
                stmt.bindDouble(5, p.close)
                stmt.executeInsert()
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    fun getOldestPriceHistoryTimestamp(): Long? {
        val cursor = readableDatabase.rawQuery(
            "SELECT MIN(timestamp) FROM price_history", null
        )
        return cursor.use { if (it.moveToFirst() && !it.isNull(0)) it.getLong(0) else null }
    }

    fun backfillHourlyPrices(candles: List<Pair<Long, Double>>): Int {
        val db = writableDatabase
        var count = 0
        db.beginTransaction()
        try {
            val stmt = db.compileStatement(
                "INSERT OR IGNORE INTO price_history (price, source, timestamp) VALUES (?, 'kraken_ohlc', ?)"
            )
            for ((ts, price) in candles) {
                stmt.clearBindings()
                stmt.bindDouble(1, price)
                stmt.bindLong(2, ts)
                stmt.executeInsert()
                count++
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
        return count
    }

    fun recordDailyPrice(date: String, open: Double, high: Double, low: Double, close: Double, volume: Double?, source: String?) {
        val cv = ContentValues().apply {
            put("date", date)
            put("open", open)
            put("high", high)
            put("low", low)
            put("close", close)
            put("volume", volume)
            put("source", source)
        }
        writableDatabase.insertWithOnConflict("daily_prices", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
    }
}

// Cursor extension helpers
private fun Cursor.getStringOrNull(index: Int): String? = if (isNull(index)) null else getString(index)
private fun Cursor.getDoubleOrNull(index: Int): Double? = if (isNull(index)) null else getDouble(index)
