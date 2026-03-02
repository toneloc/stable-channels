import Foundation
import SQLite3

/// SQLite database layer — port of src/db.rs
/// Uses raw SQLite3 C API to avoid external dependencies initially.
/// Can be migrated to GRDB later for convenience.
class DatabaseService {
    private var db: OpaquePointer?

    static let dbFilename = "stablechannels.db"

    init(dataDir: URL) throws {
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        let dbPath = dataDir.appendingPathComponent(Self.dbFilename).path

        guard sqlite3_open(dbPath, &db) == SQLITE_OK else {
            throw DatabaseError.openFailed(String(cString: sqlite3_errmsg(db)))
        }

        try initSchema()
    }

    deinit {
        sqlite3_close(db)
    }

    // MARK: - Schema

    private func initSchema() throws {
        let statements = [
            """
            CREATE TABLE IF NOT EXISTS channels (
                channel_id TEXT PRIMARY KEY,
                user_channel_id TEXT UNIQUE,
                expected_usd REAL NOT NULL DEFAULT 0.0,
                stable_sats INTEGER NOT NULL DEFAULT 0,
                note TEXT,
                receiver_sats INTEGER NOT NULL DEFAULT 0,
                latest_price REAL NOT NULL DEFAULT 0.0,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                action TEXT NOT NULL,
                amount_usd REAL NOT NULL,
                amount_btc REAL NOT NULL DEFAULT 0.0,
                btc_price REAL NOT NULL,
                fee_usd REAL NOT NULL DEFAULT 0.0,
                payment_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS payments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payment_id TEXT,
                payment_type TEXT NOT NULL DEFAULT 'manual',
                direction TEXT NOT NULL,
                amount_msat INTEGER NOT NULL,
                amount_usd REAL,
                btc_price REAL,
                counterparty TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                fee_msat INTEGER NOT NULL DEFAULT 0,
                txid TEXT,
                address TEXT,
                confirmations INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS price_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                price REAL NOT NULL,
                source TEXT,
                timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS daily_prices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL UNIQUE,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume REAL,
                source TEXT
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS onchain_txs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                txid TEXT NOT NULL,
                direction TEXT NOT NULL,
                amount_sats INTEGER NOT NULL,
                address TEXT,
                btc_price REAL,
                status TEXT NOT NULL DEFAULT 'pending',
                confirmations INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )
            """,
            "CREATE INDEX IF NOT EXISTS idx_price_history_timestamp ON price_history(timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_payments_created ON payments(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_daily_prices_date ON daily_prices(date DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_txs_created ON onchain_txs(created_at DESC)",
        ]

        for sql in statements {
            try execute(sql)
        }

        // Migrate: add receiver_sats and latest_price if missing
        let cols = try query("PRAGMA table_info(channels)")
        let colNames = cols.compactMap { $0[1] as? String }
        if !colNames.contains("receiver_sats") {
            try execute("ALTER TABLE channels ADD COLUMN receiver_sats INTEGER NOT NULL DEFAULT 0")
        }
        if !colNames.contains("latest_price") {
            try execute("ALTER TABLE channels ADD COLUMN latest_price REAL NOT NULL DEFAULT 0.0")
        }
    }

    // MARK: - Channel Operations

    func saveChannel(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        backingSats: UInt64,
        note: String?,
        receiverSats: UInt64 = 0,
        latestPrice: Double = 0.0
    ) throws {
        // Try update first
        let updateSQL = """
            UPDATE channels SET channel_id = ?, expected_usd = ?, stable_sats = ?,
                note = ?, receiver_sats = ?, latest_price = ?, updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ?
        """
        try execute(updateSQL, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(backingSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)), .real(latestPrice),
            .text(userChannelId)
        ])

        if sqlite3_changes(db) == 0 {
            let insertSQL = """
                INSERT INTO channels (channel_id, user_channel_id, expected_usd, stable_sats, note, receiver_sats, latest_price)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(channel_id) DO UPDATE SET
                    user_channel_id = excluded.user_channel_id,
                    expected_usd = excluded.expected_usd,
                    stable_sats = excluded.stable_sats,
                    note = excluded.note,
                    receiver_sats = excluded.receiver_sats,
                    latest_price = excluded.latest_price,
                    updated_at = strftime('%s', 'now')
            """
            try execute(insertSQL, params: [
                .text(channelId), .text(userChannelId), .real(expectedUSD),
                .integer(Int64(backingSats)), note.map { .text($0) } ?? .null,
                .integer(Int64(receiverSats)), .real(latestPrice)
            ])
        }
    }

    func loadChannel(userChannelId: String? = nil) throws -> ChannelRecord? {
        let sql: String
        let params: [SQLValue]
        if let id = userChannelId, !id.isEmpty {
            sql = "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, receiver_sats, latest_price FROM channels WHERE user_channel_id = ?"
            params = [.text(id)]
        } else {
            sql = "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, receiver_sats, latest_price FROM channels LIMIT 1"
            params = []
        }
        let rows = try query(sql, params: params)
        guard let row = rows.first else { return nil }

        return ChannelRecord(
            channelId: row[0] as? String ?? "",
            userChannelId: row[4] as? String ?? "",
            expectedUSD: row[1] as? Double ?? 0.0,
            note: row[2] as? String,
            backingSats: UInt64(row[3] as? Int64 ?? 0),
            receiverSats: UInt64(row[5] as? Int64 ?? 0),
            latestPrice: row[6] as? Double ?? 0.0
        )
    }

    func deleteChannel(userChannelId: String) throws {
        try execute("DELETE FROM channels WHERE user_channel_id = ?", params: [.text(userChannelId)])
    }

    // MARK: - Trade Operations

    func recordTrade(
        channelId: String,
        action: String,
        amountUSD: Double,
        amountBTC: Double,
        btcPrice: Double,
        feeUSD: Double,
        paymentId: String?,
        status: String
    ) throws -> Int64 {
        let sql = """
            INSERT INTO trades (channel_id, action, amount_usd, amount_btc, btc_price, fee_usd, payment_id, status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """
        try execute(sql, params: [
            .text(channelId), .text(action), .real(amountUSD), .real(amountBTC),
            .real(btcPrice), .real(feeUSD),
            paymentId.map { .text($0) } ?? .null, .text(status)
        ])
        return Int64(sqlite3_last_insert_rowid(db))
    }

    func getRecentTrades(limit: Int) throws -> [TradeRecord] {
        let sql = """
            SELECT id, channel_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                   payment_id, status, created_at
            FROM trades ORDER BY id DESC LIMIT ?
        """
        let rows = try query(sql, params: [.integer(Int64(limit))])
        return rows.map { row in
            TradeRecord(
                id: row[0] as? Int64 ?? 0,
                channelId: row[1] as? String ?? "",
                action: row[2] as? String ?? "",
                amountUSD: row[3] as? Double ?? 0,
                amountBTC: row[4] as? Double ?? 0,
                btcPrice: row[5] as? Double ?? 0,
                feeUSD: row[6] as? Double ?? 0,
                paymentId: row[7] as? String,
                status: row[8] as? String ?? "",
                createdAt: row[9] as? Int64 ?? 0
            )
        }
    }

    // MARK: - Payment Operations

    func recordPayment(
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        counterparty: String?,
        status: String,
        txid: String? = nil,
        address: String? = nil
    ) throws -> Int64 {
        // Dedup: skip if a payment with this payment_id already exists
        if let pid = paymentId, !pid.isEmpty {
            let existing = try query(
                "SELECT id FROM payments WHERE payment_id = ?",
                params: [.text(pid)]
            )
            if !existing.isEmpty { return existing[0][0] as? Int64 ?? 0 }
        }

        let sql = """
            INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, txid, address)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """
        try execute(sql, params: [
            paymentId.map { .text($0) } ?? .null,
            .text(paymentType), .text(direction), .integer(Int64(amountMsat)),
            amountUSD.map { .real($0) } ?? .null,
            btcPrice.map { .real($0) } ?? .null,
            counterparty.map { .text($0) } ?? .null,
            .text(status),
            txid.map { .text($0) } ?? .null,
            address.map { .text($0) } ?? .null,
        ])
        return Int64(sqlite3_last_insert_rowid(db))
    }

    func getRecentPayments(limit: Int) throws -> [PaymentRecord] {
        let sql = """
            SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
                   counterparty, status, created_at, fee_msat, txid, address, confirmations
            FROM payments ORDER BY id DESC LIMIT ?
        """
        let rows = try query(sql, params: [.integer(Int64(limit))])
        return rows.map { row in
            PaymentRecord(
                id: row[0] as? Int64 ?? 0,
                paymentId: row[1] as? String,
                paymentType: row[2] as? String ?? "manual",
                direction: row[3] as? String ?? "",
                amountMsat: UInt64(row[4] as? Int64 ?? 0),
                amountUSD: row[5] as? Double,
                btcPrice: row[6] as? Double,
                counterparty: row[7] as? String,
                status: row[8] as? String ?? "",
                createdAt: row[9] as? Int64 ?? 0,
                feeMsat: UInt64(row[10] as? Int64 ?? 0),
                txid: row[11] as? String,
                address: row[12] as? String,
                confirmations: UInt32(row[13] as? Int64 ?? 0)
            )
        }
    }

    func updateTradeStatus(_ tradeId: Int64, status: String) throws {
        try execute(
            "UPDATE trades SET status = ? WHERE id = ?",
            params: [.text(status), .integer(tradeId)]
        )
    }

    func setPendingSpliceTxid(_ txid: String) throws {
        try execute(
            "UPDATE payments SET txid = ? WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') AND txid IS NULL ORDER BY id DESC LIMIT 1",
            params: [.text(txid)]
        )
    }

    func updatePaymentStatus(paymentId: String, status: String, feeMsat: UInt64? = nil) throws {
        if let fee = feeMsat {
            try execute(
                "UPDATE payments SET status = ?, fee_msat = ? WHERE payment_id = ? AND status = 'pending'",
                params: [.text(status), .integer(Int64(fee)), .text(paymentId)]
            )
        } else {
            try execute(
                "UPDATE payments SET status = ? WHERE payment_id = ? AND status = 'pending'",
                params: [.text(status), .text(paymentId)]
            )
        }
    }

    // MARK: - Price History

    func recordPrice(_ price: Double, source: String? = nil) throws {
        try execute(
            "INSERT INTO price_history (price, source) VALUES (?, ?)",
            params: [.real(price), source.map { .text($0) } ?? .null]
        )
    }

    func getPriceHistory(hours: UInt32) throws -> [PriceRecord] {
        let cutoff = Int64(Date().timeIntervalSince1970) - Int64(hours) * 3600
        let sql = """
            SELECT id, price, source, timestamp FROM price_history
            WHERE timestamp > ? ORDER BY timestamp ASC
        """
        let rows = try query(sql, params: [.integer(cutoff)])
        return rows.map { row in
            PriceRecord(
                id: row[0] as? Int64 ?? 0,
                price: row[1] as? Double ?? 0,
                source: row[2] as? String,
                timestamp: row[3] as? Int64 ?? 0
            )
        }
    }

    func getDailyPrices(days: UInt32) throws -> [DailyPriceRecord] {
        let calendar = Calendar.current
        let cutoffDate = calendar.date(byAdding: .day, value: -Int(days), to: Date()) ?? Date()
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        let cutoffStr = formatter.string(from: cutoffDate)

        let sql = """
            SELECT date, open, high, low, close, volume FROM daily_prices
            WHERE date >= ? ORDER BY date ASC
        """
        let rows = try query(sql, params: [.text(cutoffStr)])
        return rows.map { row in
            DailyPriceRecord(
                date: row[0] as? String ?? "",
                open: row[1] as? Double ?? 0,
                high: row[2] as? Double ?? 0,
                low: row[3] as? Double ?? 0,
                close: row[4] as? Double ?? 0,
                volume: row[5] as? Double
            )
        }
    }

    func recordDailyPrice(
        date: String, open: Double, high: Double, low: Double,
        close: Double, volume: Double?, source: String? = nil
    ) throws {
        try execute(
            "INSERT OR REPLACE INTO daily_prices (date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params: [
                .text(date), .real(open), .real(high), .real(low), .real(close),
                volume.map { .real($0) } ?? .null,
                source.map { .text($0) } ?? .null
            ]
        )
    }

    // MARK: - Raw SQLite Helpers

    private enum SQLValue {
        case text(String)
        case integer(Int64)
        case real(Double)
        case null
    }

    /// SQLITE_TRANSIENT tells SQLite to make its own copy of the string data immediately,
    /// preventing use-after-free when the Swift string's backing buffer is deallocated.
    private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    private func bindParams(_ stmt: OpaquePointer?, params: [SQLValue]) {
        for (i, param) in params.enumerated() {
            let idx = Int32(i + 1)
            switch param {
            case .text(let s):
                _ = s.withCString { cStr in
                    sqlite3_bind_text(stmt, idx, cStr, -1, SQLITE_TRANSIENT)
                }
            case .integer(let n): sqlite3_bind_int64(stmt, idx, n)
            case .real(let d): sqlite3_bind_double(stmt, idx, d)
            case .null: sqlite3_bind_null(stmt, idx)
            }
        }
    }

    private func execute(_ sql: String, params: [SQLValue] = []) throws {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            throw DatabaseError.prepareFailed(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }

        bindParams(stmt, params: params)

        let result = sqlite3_step(stmt)
        guard result == SQLITE_DONE || result == SQLITE_ROW else {
            throw DatabaseError.executeFailed(String(cString: sqlite3_errmsg(db)))
        }
    }

    private func query(_ sql: String, params: [SQLValue] = []) throws -> [[Any?]] {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            throw DatabaseError.prepareFailed(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }

        bindParams(stmt, params: params)

        var rows: [[Any?]] = []
        let colCount = sqlite3_column_count(stmt)
        while sqlite3_step(stmt) == SQLITE_ROW {
            var row: [Any?] = []
            for col in 0..<colCount {
                switch sqlite3_column_type(stmt, col) {
                case SQLITE_INTEGER: row.append(sqlite3_column_int64(stmt, col))
                case SQLITE_FLOAT: row.append(sqlite3_column_double(stmt, col))
                case SQLITE_TEXT: row.append(String(cString: sqlite3_column_text(stmt, col)))
                case SQLITE_NULL: row.append(nil)
                default: row.append(nil)
                }
            }
            rows.append(row)
        }
        return rows
    }
}

enum DatabaseError: LocalizedError {
    case openFailed(String)
    case prepareFailed(String)
    case executeFailed(String)

    var errorDescription: String? {
        switch self {
        case .openFailed(let msg): return "Database open failed: \(msg)"
        case .prepareFailed(let msg): return "SQL prepare failed: \(msg)"
        case .executeFailed(let msg): return "SQL execute failed: \(msg)"
        }
    }
}
