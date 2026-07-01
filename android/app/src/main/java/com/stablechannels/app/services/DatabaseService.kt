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

class DatabaseService(context: Context) : SQLiteOpenHelper(
    context,
    File(Constants.userDataDir(context), DB_FILENAME).absolutePath,
    null,
    DB_VERSION
) {
    companion object {
        private const val DB_FILENAME = "stablechannels.db"
        private const val DB_VERSION = 2
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

        db.execSQL("CREATE INDEX IF NOT EXISTS idx_price_history_ts ON price_history(timestamp)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_payments_created ON payments(created_at)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_trades_created ON trades(created_at)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_onchain_txs_created ON onchain_txs(created_at)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) {
            db.execSQL("ALTER TABLE channels ADD COLUMN receiver_sats INTEGER NOT NULL DEFAULT 0")
            db.execSQL("ALTER TABLE channels ADD COLUMN latest_price REAL NOT NULL DEFAULT 0.0")
        }
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
            "SELECT channel_id, user_channel_id, expected_usd, note, stable_sats, receiver_sats, latest_price FROM channels WHERE user_channel_id = ?",
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
                    latestPrice = it.getDouble(6)
                )
            } else null
        }
    }

    fun deleteChannel(userChannelId: String) {
        writableDatabase.delete("channels", "user_channel_id = ?", arrayOf(userChannelId))
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
                            ?: throw IllegalStateException("No channel row for user_channel_id=$ucid")
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
                val stmt = db.compileStatement(
                    "UPDATE channels SET stable_sats = stable_sats + ?, updated_at = strftime('%s','now') WHERE user_channel_id = ? AND stable_sats + ? >= 0"
                )
                stmt.bindLong(1, backingDeltaSats)
                stmt.bindString(2, ucid)
                stmt.bindLong(3, backingDeltaSats)
                val rows = stmt.executeUpdateDelete()
                if (rows != 1) {
                    throw IllegalStateException(
                        "backing UPDATE affected $rows rows for user_channel_id=$ucid"
                    )
                }
                resultingBacking = readBackingSats(db, ucid)
                    ?: throw IllegalStateException("No channel row for user_channel_id=$ucid")
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

    fun getRecentPayments(limit: Int = 50): List<PaymentRecord> {
        val cursor = readableDatabase.rawQuery(
            "SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, created_at, fee_msat, txid, address, confirmations FROM payments ORDER BY created_at DESC LIMIT ?",
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

    fun getPendingChannelClosePaymentId(): String? {
        val cursor = readableDatabase.rawQuery(
            "SELECT payment_id FROM payments WHERE payment_type = 'channel_close' AND status = 'pending' ORDER BY created_at DESC LIMIT 1",
            null
        )
        return cursor.use { if (it.moveToFirst()) it.getString(0) else null }
    }

    fun setPendingSpliceTxid(txid: String) {
        writableDatabase.execSQL(
            "UPDATE payments SET txid = ?, status = 'completed' WHERE rowid = (SELECT rowid FROM payments WHERE payment_type IN ('splice_in','splice_out') AND status = 'pending' ORDER BY created_at DESC LIMIT 1)",
            arrayOf(txid)
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
        // Auto-expire pending splices: 10 min if no txid, 1 hour if has txid
        val noTxidCutoff = System.currentTimeMillis() / 1000 - 600
        writableDatabase.execSQL(
            "UPDATE payments SET status = 'failed' WHERE status = 'pending' AND payment_type IN ('splice_in','splice_out') AND txid IS NULL AND created_at < ?",
            arrayOf(noTxidCutoff)
        )
        val txidCutoff = System.currentTimeMillis() / 1000 - 3600
        writableDatabase.execSQL(
            "UPDATE payments SET status = 'failed' WHERE status = 'pending' AND payment_type IN ('splice_in','splice_out') AND txid IS NOT NULL AND created_at < ?",
            arrayOf(txidCutoff)
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
