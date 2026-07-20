import Foundation
import SQLite3

struct PaymentPersistenceResult {
    let isNewPayment: Bool
    let backingSats: UInt64?
}

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
        // Main app and NSE genuinely overlap on this DB — wait briefly for locks
        // instead of failing instantly with SQLITE_BUSY.
        sqlite3_busy_timeout(db, 2000)

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
                resolution_id INTEGER,
                tx_block_height INTEGER,
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
            """
            CREATE TABLE IF NOT EXISTS pending_operations (
                op_id TEXT PRIMARY KEY NOT NULL,
                op_type TEXT NOT NULL,
                funding_outpoint_txid TEXT,
                funding_outpoint_vout INTEGER,
                closing_txid TEXT,
                balance_sats INTEGER,
                balance_usd REAL,
                btc_price REAL,
                counterparty TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                resolved_at INTEGER
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS pending_stability_send (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                payment_id TEXT NOT NULL,
                amount_msat INTEGER NOT NULL,
                price REAL NOT NULL,
                created_at INTEGER NOT NULL
            )
            """,
            """
            CREATE TABLE IF NOT EXISTS onchain_receive_txids (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                address TEXT NOT NULL,
                txid TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                resolved_at INTEGER
            )
            """,
            "CREATE INDEX IF NOT EXISTS idx_price_history_timestamp ON price_history(timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_pending_operations_status ON pending_operations(status)",
            "CREATE INDEX IF NOT EXISTS idx_payments_created ON payments(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_daily_prices_date ON daily_prices(date DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_txs_created ON onchain_txs(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_receive_txids_status ON onchain_receive_txids(status)"
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
        if !colNames.contains("native_sats") {
            try execute("ALTER TABLE channels ADD COLUMN native_sats INTEGER NOT NULL DEFAULT 0")
        }

        // Migrate: add tx_block_height to payments if missing (on-chain confirmation tracking)
        let paymentsCols = try query("PRAGMA table_info(payments)")
        let paymentsColNames = paymentsCols.compactMap { $0[1] as? String }
        if !paymentsColNames.contains("tx_block_height") {
            try execute("ALTER TABLE payments ADD COLUMN tx_block_height INTEGER")
        }

        // Migrate: add resolution_id to payments if missing (onchain deposit <-> resolver link)
        if !paymentsColNames.contains("resolution_id") {
            try execute("ALTER TABLE payments ADD COLUMN resolution_id INTEGER")
        }
    }

    // MARK: - Channel Operations

    func saveChannel(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        backingSats: UInt64,
        nativeSats: UInt64 = 0,
        note: String?,
        receiverSats: UInt64 = 0,
        latestPrice: Double = 0.0
    ) throws {
        // Try update first
        let updateSQL = """
            UPDATE channels SET channel_id = ?, expected_usd = ?, stable_sats = ?,
                native_sats = ?, note = ?, receiver_sats = ?, latest_price = ?, updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ?
        """
        try execute(updateSQL, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(backingSats)),
            .integer(Int64(nativeSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)), .real(latestPrice),
            .text(userChannelId)
        ])

        if sqlite3_changes(db) == 0 {
            let insertSQL = """
                INSERT INTO channels (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, note, receiver_sats, latest_price)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(channel_id) DO UPDATE SET
                    user_channel_id = excluded.user_channel_id,
                    expected_usd = excluded.expected_usd,
                    stable_sats = excluded.stable_sats,
                    native_sats = excluded.native_sats,
                    note = excluded.note,
                    receiver_sats = excluded.receiver_sats,
                    latest_price = excluded.latest_price,
                    updated_at = strftime('%s', 'now')
            """
            try execute(insertSQL, params: [
                .text(channelId), .text(userChannelId), .real(expectedUSD),
                .integer(Int64(backingSats)), .integer(Int64(nativeSats)),
                note.map { .text($0) } ?? .null,
                .integer(Int64(receiverSats)), .real(latestPrice)
            ])
        }
    }

    /// Persist channel metadata without touching stable_sats.
    ///
    /// Incoming stability payments update stable_sats transactionally. Excluding it from this
    /// follow-up write prevents stale in-memory state from undoing a concurrent DB increment.
    func saveChannelPreservingBacking(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        nativeSats: UInt64 = 0,
        note: String?,
        receiverSats: UInt64 = 0,
        latestPrice: Double = 0.0
    ) throws {
        let sql = """
            UPDATE channels SET channel_id = ?, expected_usd = ?, native_sats = ?, note = ?,
                receiver_sats = ?, latest_price = ?, updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ?
        """
        try execute(sql, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(nativeSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)),
            .real(latestPrice), .text(userChannelId)
        ])
        let changedRows = sqlite3_changes(db)
        guard changedRows == 1 else {
            throw DatabaseError.executeFailed(
                "channel metadata UPDATE affected \(changedRows) rows for user_channel_id=\(userChannelId)"
            )
        }
    }

    func loadChannel(userChannelId: String? = nil) throws -> ChannelRecord? {
        let sql: String
        let params: [SQLValue]
        if let id = userChannelId, !id.isEmpty {
            sql = "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, receiver_sats, latest_price, native_sats FROM channels WHERE user_channel_id = ?"
            params = [.text(id)]
        } else {
            /// When multiple rows exist, pick a deterministic latest record (not “active channel” semantics).
            sql = """
                SELECT channel_id, expected_usd, note, stable_sats, user_channel_id,
                       receiver_sats, latest_price, native_sats
                FROM channels
                ORDER BY updated_at DESC, channel_id DESC
                LIMIT 1
            """
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
            nativeSats: UInt64(row[7] as? Int64 ?? 0),
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
    ) throws -> Bool {
        // Dedup: skip if a payment with this payment_id already exists
        if let pid = paymentId, !pid.isEmpty {
            let existing = try query(
                "SELECT id FROM payments WHERE payment_id = ?",
                params: [.text(pid)]
            )
            if !existing.isEmpty {
                return false
            }
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
            address.map { .text($0) } ?? .null
        ])
        return true
    }

    /// Insert a payment and atomically update channel backing sats in one SQLite transaction.
    /// Returns whether the payment was new and the authoritative backing value, when applicable.
    func recordPaymentAndMaybeUpdateBacking(
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        status: String,
        userChannelId: String?,
        backingDeltaSats: Int64?
    ) throws -> PaymentPersistenceResult {
        try execute("BEGIN IMMEDIATE")
        do {
            if let pid = paymentId, !pid.isEmpty {
                let existing = try query("SELECT id FROM payments WHERE payment_id = ?", params: [.text(pid)])
                if !existing.isEmpty {
                    let backing = try authoritativeBacking(
                        userChannelId: userChannelId,
                        required: backingDeltaSats != nil
                    )
                    try execute("ROLLBACK")
                    return PaymentPersistenceResult(isNewPayment: false, backingSats: backing)
                }
            }
            try execute(
                "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params: [
                    paymentId.map { .text($0) } ?? .null,
                    .text(paymentType), .text(direction), .integer(Int64(amountMsat)),
                    amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    .text(status)
                ]
            )
            var resultingBacking: UInt64?
            if let delta = backingDeltaSats {
                guard let ucid = userChannelId, !ucid.isEmpty else {
                    throw DatabaseError.executeFailed("userChannelId required for backing update")
                }
                let rows = try query(
                    "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
                    params: [.text(ucid)]
                )
                guard let current = rows.first?[0] as? Int64 else {
                    throw DatabaseError.missingChannelRow(ucid)
                }
                // Clamp instead of refusing: this runs after the payment already
                // happened, so recording reality beats wedging reconcile forever.
                let newBacking = max(0, current + delta)
                if current + delta < 0 {
                    AuditService.log("BACKING_CLAMPED", data: [
                        "user_channel_id": ucid,
                        "current_backing_sats": "\(current)",
                        "delta_sats": "\(delta)"
                    ])
                }
                try execute(
                    "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s', 'now') WHERE user_channel_id = ?",
                    params: [.integer(newBacking), .text(ucid)]
                )
                let changedRows = sqlite3_changes(db)
                if changedRows != 1 {
                    throw DatabaseError.executeFailed(
                        "backing UPDATE affected \(changedRows) rows for user_channel_id=\(ucid)"
                    )
                }
                resultingBacking = UInt64(newBacking)
            }
            try execute("COMMIT")
            return PaymentPersistenceResult(
                isNewPayment: true,
                backingSats: resultingBacking
            )
        } catch {
            try? execute("ROLLBACK")
            throw error
        }
    }

    private func authoritativeBacking(
        userChannelId: String?,
        required: Bool
    ) throws -> UInt64? {
        guard required else { return nil }
        guard let ucid = userChannelId, !ucid.isEmpty else {
            throw DatabaseError.executeFailed("userChannelId required to load backing")
        }
        let rows = try query(
            "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
            params: [.text(ucid)]
        )
        guard let value = rows.first?[0] as? Int64 else {
            throw DatabaseError.missingChannelRow(ucid)
        }
        guard value >= 0 else {
            throw DatabaseError.executeFailed(
                "No valid backing row for user_channel_id=\(ucid)"
            )
        }
        return UInt64(value)
    }

    // MARK: - Pending Stability Send

    /// Durable cross-process marker for an in-flight outgoing stability payment.
    /// Claimed under BEGIN IMMEDIATE so the foreground timer and the NSE can never
    /// both hold the send slot at once.
    ///
    /// Returns true if this caller claimed the slot; false if a marker already
    /// exists (another sender owns it) or the claim could not be persisted.
    func claimPendingSend(amountMsat: UInt64, price: Double) -> Bool {
        do {
            try execute("BEGIN IMMEDIATE")
        } catch {
            return false
        }
        do {
            let existing = try query("SELECT id FROM pending_stability_send WHERE id = 1")
            guard existing.isEmpty else {
                try execute("ROLLBACK")
                return false
            }
            try execute(
                "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)",
                params: [
                    .integer(Int64(amountMsat)),
                    .real(price),
                    .integer(Int64(Date().timeIntervalSince1970))
                ]
            )
            try execute("COMMIT")
            return true
        } catch {
            try? execute("ROLLBACK")
            return false
        }
    }

    /// Attach the real payment id to the claimed send marker once the keysend returns.
    @discardableResult
    func setPendingSendPaymentId(_ paymentId: String) -> Bool {
        do {
            try execute(
                "UPDATE pending_stability_send SET payment_id = ? WHERE id = 1",
                params: [.text(paymentId)]
            )
            return true
        } catch {
            return false
        }
    }

    func loadPendingSend() -> PendingStabilitySend? {
        guard let rows = try? query(
            "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1"
        ), let row = rows.first else {
            return nil
        }
        return PendingStabilitySend(
            paymentId: row[0] as? String ?? "",
            amountMsat: UInt64(row[1] as? Int64 ?? 0),
            price: row[2] as? Double ?? 0,
            createdAt: row[3] as? Int64 ?? 0
        )
    }

    func clearPendingSend() {
        try? execute("DELETE FROM pending_stability_send WHERE id = 1")
    }

    private func paymentRecord(from row: [Any?]) -> PaymentRecord {
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
            confirmations: UInt32(row[13] as? Int64 ?? 0),
            txBlockHeight: (row[14] as? Int64).flatMap(UInt32.init)
        )
    }

    /// Returns the single most recent received payment, or nil if none exists.
    /// Used by the home status bubble to navigate to payment details.
    func latestReceivedPayment() -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE direction = "received"
        AND NOT (payment_type = 'lightning' AND amount_msat < 1000)
        ORDER BY id DESC LIMIT 1
        """
        guard let row = try? query(sql, params: []).first else { return nil }
        return paymentRecord(from: row)
    }

    func paymentsNeedingConfirmation(currentBlockHeight: UInt32) throws -> [PaymentRecord] {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE txid IS NOT NULL
        AND txid != ''
        AND payment_type IN ('onchain', 'splice_in', 'splice_out', 'channel_close')
        AND status != 'failed'
        AND (tx_block_height IS NULL OR tx_block_height + ? > ?)
        ORDER BY created_at DESC
        LIMIT 50
        """
        let rows = try query(
            sql,
            params: [.integer(Int64(ConfirmationPolicy.requiredConfirmations - 1)), .integer(Int64(currentBlockHeight))]
        )
        return rows.compactMap { row in
            guard row.count >= 15 else { return nil }
            return paymentRecord(from: row)
        }
    }

    func getPayment(byId id: Int64) throws -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments WHERE id = ? LIMIT 1
        """
        let rows = try query(sql, params: [.integer(id)])
        guard let row = rows.first, row.count >= 15 else { return nil }
        return paymentRecord(from: row)
    }

    func updateConfirmations(paymentId: Int64, txBlockHeight: UInt32, currentBlockHeight: UInt32) throws {
        let confs = max(Int(currentBlockHeight) - Int(txBlockHeight) + 1, 0)
        try execute(
            "UPDATE payments SET tx_block_height = ?, confirmations = ? WHERE id = ?",
            params: [.integer(Int64(txBlockHeight)), .integer(Int64(confs)), .integer(paymentId)]
        )
    }

    func getRecentPayments(limit: Int) throws -> [PaymentRecord] {
        let sql = """
            SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
                   counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
            FROM payments
            WHERE NOT (payment_type = 'lightning' AND amount_msat < 1000)
            ORDER BY id DESC LIMIT ?
        """
        let rows = try query(sql, params: [.integer(Int64(limit))])
        return rows.map { row in
            paymentRecord(from: row)
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
            """
            UPDATE payments
            SET txid = ?
            WHERE payment_type = 'splice_in'
              AND status IN ('pending', 'failed')
              AND txid IS NULL
            ORDER BY id DESC LIMIT 1
            """,
            params: [.text(txid)]
        )
    }

    func getPendingSpliceTxid() throws -> String? {
        let rows = try query(
            "SELECT txid FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') AND txid IS NOT NULL ORDER BY id DESC LIMIT 1"
        )
        return rows.first?[0] as? String
    }

    func hasPendingSplice() throws -> Bool {
        // If the app died before SpliceNegotiated delivered a txid, there is no
        // durable in-flight splice to wait for. Let that pre-negotiation lock heal.
        // Keep with-txid rows pending: confirmation can outlive the app process,
        // and the splice confirmation monitor completes them after 1 conf.
        let noTxidCutoff = Int64(Date().timeIntervalSince1970) - 600
        try execute(
            """
            UPDATE payments
            SET status = 'failed'
            WHERE status = 'pending'
              AND payment_type IN ('splice_in', 'splice_out')
              AND txid IS NULL
              AND created_at < ?
            """,
            params: [.integer(noTxidCutoff)]
        )
        let rows = try query(
            "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') LIMIT 1"
        )
        return !rows.isEmpty
    }

    @discardableResult
    func completeLatestSplice(txid: String?) -> Bool {
        do {
            if let txid, !txid.isEmpty {
                try execute(
                    """
                    UPDATE payments
                    SET status = 'completed'
                    WHERE payment_type IN ('splice_in', 'splice_out')
                      AND txid = ?
                      AND status IN ('pending', 'failed')
                    """,
                    params: [.text(txid)]
                )
            } else {
                try execute(
                    """
                    UPDATE payments
                    SET status = 'completed'
                    WHERE payment_type IN ('splice_in', 'splice_out')
                      AND status IN ('pending', 'failed')
                    ORDER BY id DESC LIMIT 1
                    """
                )
            }
            return true
        } catch {
            return false
        }
    }

    /// Returns true only if a splice row was actually flipped to completed,
    /// so callers can use the result as the "this ChannelReady was a splice" signal.
    @discardableResult
    func completeSplice(txid: String) -> Bool {
        do {
            try execute(
                """
                UPDATE payments
                SET status = 'completed', confirmations = 1
                WHERE payment_type IN ('splice_in', 'splice_out')
                  AND txid = ?
                  AND status IN ('pending', 'failed')
                """,
                params: [.text(txid)]
            )
            return sqlite3_changes(db) > 0
        } catch {
            return false
        }
    }

    @discardableResult
    func failLatestPendingSplice() -> Bool {
        do {
            try execute(
                """
                UPDATE payments
                SET status = 'failed'
                WHERE payment_type IN ('splice_in', 'splice_out')
                  AND status = 'pending'
                ORDER BY id DESC LIMIT 1
                """
            )
            return true
        } catch {
            return false
        }
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

    func isOutgoingStabilityPayment(paymentId: String) throws -> Bool {
        let rows = try query(
            "SELECT 1 FROM payments WHERE payment_id = ? AND payment_type = 'stability' AND direction = 'sent' LIMIT 1",
            params: [.text(paymentId)]
        )
        return !rows.isEmpty
    }

    // MARK: - Pending Operations

    @discardableResult
    func insertPendingOperation(
        opId: String,
        opType: String,
        fundingOutpointTxid: String?,
        fundingOutpointVout: UInt32?,
        balanceSats: UInt64? = nil,
        balanceUsd: Double? = nil,
        btcPrice: Double? = nil,
        counterparty: String? = nil
    ) -> Bool {
        do {
            try execute(
                """
                INSERT INTO pending_operations
                    (op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                     balance_sats, balance_usd, btc_price, counterparty, status)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending')
                ON CONFLICT(op_id) DO UPDATE SET
                    op_type = excluded.op_type,
                    funding_outpoint_txid = excluded.funding_outpoint_txid,
                    funding_outpoint_vout = excluded.funding_outpoint_vout,
                    balance_sats = excluded.balance_sats,
                    balance_usd = excluded.balance_usd,
                    btc_price = excluded.btc_price,
                    counterparty = excluded.counterparty,
                    status = 'pending'
                """,
                params: [
                    .text(opId),
                    .text(opType),
                    fundingOutpointTxid.map { .text($0) } ?? .null,
                    fundingOutpointVout.map { .integer(Int64($0)) } ?? .null,
                    balanceSats.map { .integer(Int64($0)) } ?? .null,
                    balanceUsd.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    counterparty.map { .text($0) } ?? .null
                ]
            )
            return true
        } catch {
            return false
        }
    }

    /// Update a pending_operations row, only if it is still in 'pending' state.
    /// Used by the resolver so a stale update can't clobber a row that's
    /// already been marked resolved/failed by a parallel attempt.
    @discardableResult
    func updatePendingOperation(opId: String, closingTxid: String, status: String) -> Bool {
        do {
            try execute(
                """
                UPDATE pending_operations
                SET closing_txid = ?, status = ?, resolved_at = strftime('%s', 'now')
                WHERE op_id = ? AND status = 'pending'
                """,
                params: [.text(closingTxid), .text(status), .text(opId)]
            )
            return sqlite3_changes(db) > 0
        } catch {
            return false
        }
    }

    func fetchPendingOperations() -> [PendingOperation] {
        do {
            let rows = try query(
                """
                SELECT op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                       closing_txid, balance_sats, balance_usd, btc_price, counterparty,
                       status, created_at, resolved_at
                FROM pending_operations
                WHERE status = 'pending'
                """
            )
            return rows.map { Self.parsePendingOperation($0) }
        } catch {
            return []
        }
    }

    /// Fetch a single pending_operations row by opId. Uses the primary key
    /// index for an O(1) lookup instead of a full table scan.
    func fetchPendingOperation(opId: String) -> PendingOperation? {
        do {
            let rows = try query(
                """
                SELECT op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                       closing_txid, balance_sats, balance_usd, btc_price, counterparty,
                       status, created_at, resolved_at
                FROM pending_operations
                WHERE op_id = ?
                LIMIT 1
                """,
                params: [.text(opId)]
            )
            return rows.first.map { Self.parsePendingOperation($0) }
        } catch {
            return nil
        }
    }

    // MARK: - Onchain Receive Resolutions

    @discardableResult
    func insertOnchainReceiveResolution(address: String) -> Int64? {
        do {
            try execute(
                """
                INSERT INTO onchain_receive_txids (address, status)
                VALUES (?, 'pending')
                """,
                params: [.text(address)]
            )
            return Int64(sqlite3_last_insert_rowid(db))
        } catch {
            AuditService.log("DB_INSERT_RECEIVE_RES_FAILED", data: ["error": "\(error)"])
            return nil
        }
    }

    func fetchPendingOnchainReceives() -> [OnchainReceiveResolution] {
        do {
            let rows = try query(
                """
                SELECT id, address, txid, status, created_at, resolved_at
                FROM onchain_receive_txids
                WHERE status = 'pending'
                ORDER BY created_at ASC
                """,
                params: []
            )
            return rows.map { row in
                OnchainReceiveResolution(
                    id: row[0] as? Int64 ?? 0,
                    address: row[1] as? String ?? "",
                    txid: row[2] as? String,
                    status: row[3] as? String ?? "pending",
                    createdAt: row[4] as? Int64 ?? 0,
                    resolvedAt: row[5] as? Int64
                )
            }
        } catch {
            return []
        }
    }

    @discardableResult
    func updateOnchainReceiveResolution(id: Int64, txid: String) -> Bool {
        do {
            try execute(
                """
                UPDATE onchain_receive_txids
                SET txid = ?, status = 'resolved', resolved_at = strftime('%s', 'now')
                WHERE id = ? AND status = 'pending'
                """,
                params: [.text(txid), .integer(id)]
            )
            return sqlite3_changes(db) > 0
        } catch {
            return false
        }
    }

    /// Read pending onchain received payments (no txid yet). Used by the
    /// AppState to find the row to back-fill once the resolver returns a
    /// real txid. FIFO order (oldest first).
    func fetchPendingOnchainReceiveRows() -> [PendingOnchainPayment] {
        do {
            let rows = try query(
                """
                SELECT payment_id, amount_msat, created_at
                FROM payments
                WHERE payment_type = 'onchain'
                  AND direction = 'received'
                  AND status = 'pending'
                ORDER BY created_at ASC
                """,
                params: []
            )
            return rows.map { row in
                PendingOnchainPayment(
                    paymentId: row[0] as? String ?? "",
                    amountMsat: row[1] as? Int64 ?? 0,
                    createdAt: row[2] as? Int64 ?? 0
                )
            }
        } catch {
            return []
        }
    }

    /// Update a payments row with a real txid and status. Used by the
    /// AppState once the onchain resolver hits Esplora and we know the
    /// real receiving txid.
    @discardableResult
    func updatePaymentTxid(paymentId: String, txid: String, status: String) -> Bool {
        do {
            try execute(
                """
                UPDATE payments
                SET txid = ?, status = ?
                WHERE payment_id = ?
                """,
                params: [.text(txid), .text(status), .text(paymentId)]
            )
            return true
        } catch {
            return false
        }
    }

    /// Set the `resolution_id` link on a payments row. Used to wire a
    /// `payments` row to its corresponding `onchain_receive_txids` resolver row.
    @discardableResult
    func updatePaymentResolution(paymentId: String, resolutionId: Int64) -> Bool {
        do {
            try execute(
                "UPDATE payments SET resolution_id = ? WHERE payment_id = ?",
                params: [.integer(resolutionId), .text(paymentId)]
            )
            return true
        } catch {
            return false
        }
    }

    /// Insert a pending onchain-received `payments` row that is pre-linked
    /// to a freshly-created `onchain_receive_txids` resolution. Use this
    /// (instead of `recordPayment` + `updatePaymentResolution`) so a crash
    /// between the two writes cannot leave an orphan row.
    @discardableResult
    func recordOnchainPaymentWithResolution(
        paymentId: String,
        amountMsat: Int64,
        amountUSD: Double?,
        btcPrice: Double?,
        resolutionId: Int64
    ) -> Bool {
        do {
            try execute(
                """
                INSERT INTO payments (
                    payment_id, payment_type, direction, amount_msat,
                    amount_usd, btc_price, status, created_at, resolution_id
                )
                VALUES (?, 'onchain', 'received', ?, ?, ?, 'pending', strftime('%s', 'now'), ?)
                """,
                params: [
                    .text(paymentId),
                    .integer(amountMsat),
                    amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    .integer(resolutionId)
                ]
            )
            return true
        } catch {
            AuditService.log("DB_INSERT_ONCHAIN_PAYMENT_FAILED", data: ["error": "\(error)"])
            return false
        }
    }

    /// Fetch the single pending onchain-received payments row that was
    /// created with the given `resolutionId`. Used to back-fill the row
    /// with the real txid once the resolver succeeds.
    func fetchPendingOnchainReceiveRow(resolutionId: Int64) -> PendingOnchainPayment? {
        do {
            let rows = try query(
                """
                SELECT payment_id, amount_msat, created_at
                FROM payments
                WHERE payment_type = 'onchain'
                  AND direction = 'received'
                  AND status = 'pending'
                  AND resolution_id = ?
                ORDER BY created_at ASC
                LIMIT 1
                """,
                params: [.integer(resolutionId)]
            )
            guard let row = rows.first else { return nil }
            return PendingOnchainPayment(
                paymentId: row[0] as? String ?? "",
                amountMsat: row[1] as? Int64 ?? 0,
                createdAt: row[2] as? Int64 ?? 0
            )
        } catch {
            return nil
        }
    }

    /// Fetch the most recent *resolved* onchain receive txid. Used by
    /// `AppState.handleOnchainReceiveResolved` so the UI shows the latest
    /// resolved txid (not just the one that fired the callback). Tiebreak
    /// by `id DESC` so two resolutions in the same second are deterministic.
    func fetchLatestResolvedOnchainTxid() -> String? {
        do {
            let rows = try query(
                """
                SELECT txid FROM onchain_receive_txids
                WHERE status = 'resolved' AND txid IS NOT NULL
                ORDER BY resolved_at DESC, id DESC LIMIT 1
                """,
                params: []
            )
            return rows.first?.first as? String
        } catch {
            return nil
        }
    }

    /// Delete a pending `onchain_receive_txids` row. Used to roll back the
    /// resolver-side row if the linked `payments` row insert fails.
    @discardableResult
    func deleteOnchainReceiveResolution(id: Int64) -> Bool {
        do {
            try execute(
                "DELETE FROM onchain_receive_txids WHERE id = ?",
                params: [.integer(id)]
            )
            return true
        } catch {
            return false
        }
    }

    private static func parsePendingOperation(_ row: [Any?]) -> PendingOperation {
        PendingOperation(
            opId: row[0] as? String ?? "",
            opType: row[1] as? String ?? "",
            fundingOutpointTxid: row[2] as? String,
            fundingOutpointVout: row[3].flatMap { ($0 as? Int64).map { UInt32($0) } },
            closingTxid: row[4] as? String,
            balanceSats: row[5].flatMap { ($0 as? Int64).map { UInt64($0) } },
            balanceUsd: row[6] as? Double,
            btcPrice: row[7] as? Double,
            counterparty: row[8] as? String,
            status: row[9] as? String ?? "pending",
            createdAt: row[10] as? Int64 ?? 0,
            resolvedAt: row[11] as? Int64
        )
    }

    // MARK: - Price History

    func recordPrice(_ price: Double, source: String? = nil) throws {
        try execute(
            "INSERT INTO price_history (price, source) VALUES (?, ?)",
            params: [.real(price), source.map { .text($0) } ?? .null]
        )
    }

    /// Bulk-insert hourly price records, skipping any timestamps that already exist within ±30 min.
    func backfillHourlyPrices(_ prices: [(timestamp: Int64, price: Double)]) throws -> Int {
        var count = 0
        for (ts, price) in prices {
            // Skip if a record already exists within 30 minutes of this timestamp
            let existing = try query(
                "SELECT 1 FROM price_history WHERE timestamp BETWEEN ? AND ? LIMIT 1",
                params: [.integer(ts - 1800), .integer(ts + 1800)]
            )
            if existing.isEmpty {
                try execute(
                    "INSERT INTO price_history (price, source, timestamp) VALUES (?, 'kraken_ohlc', ?)",
                    params: [.real(price), .integer(ts)]
                )
                count += 1
            }
        }
        return count
    }

    func getOldestPriceHistoryTimestamp() throws -> Int64? {
        let rows = try query("SELECT MIN(timestamp) FROM price_history")
        return rows.first?[0] as? Int64
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

    func bulkInsertDailyPrices(_ prices: [(String, Double, Double, Double, Double, Double?)]) throws -> Int {
        var count = 0
        for (date, open, high, low, close, volume) in prices {
            try execute(
                "INSERT OR IGNORE INTO daily_prices (date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, 'seed')",
                params: [
                    .text(date), .real(open), .real(high), .real(low), .real(close),
                    volume.map { .real($0) } ?? .null
                ]
            )
            count += 1
        }
        return count
    }

    func getOldestDailyPriceDate() throws -> String? {
        let rows = try query("SELECT date FROM daily_prices ORDER BY date ASC LIMIT 1", params: [])
        return rows.first?[0] as? String
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
    /// No channels row exists for the given user_channel_id — recoverable by
    /// recreating the row from in-memory state, unlike a plain execute failure.
    case missingChannelRow(String)

    var errorDescription: String? {
        switch self {
        case .openFailed(let msg): return "Database open failed: \(msg)"
        case .prepareFailed(let msg): return "SQL prepare failed: \(msg)"
        case .executeFailed(let msg): return "SQL execute failed: \(msg)"
        case .missingChannelRow(let ucid): return "No channel row for user_channel_id=\(ucid)"
        }
    }
}

/// Durable marker row for an in-flight outgoing stability payment.
/// `paymentId` is empty between the claim and the keysend returning an id.
struct PendingStabilitySend: Equatable {
    let paymentId: String
    let amountMsat: UInt64
    let price: Double
    let createdAt: Int64
}

struct OnchainReceiveResolution: Hashable {
    let id: Int64
    let address: String
    let txid: String?
    let status: String
    let createdAt: Int64
    let resolvedAt: Int64?
}

struct PendingOnchainPayment: Hashable {
    let paymentId: String
    let amountMsat: Int64
    let createdAt: Int64
}

struct PendingOperation: Equatable {
    let opId: String
    let opType: String
    let fundingOutpointTxid: String?
    let fundingOutpointVout: UInt32?
    let closingTxid: String?
    let balanceSats: UInt64?
    let balanceUsd: Double?
    let btcPrice: Double?
    let counterparty: String?
    let status: String
    let createdAt: Int64
    let resolvedAt: Int64?
}
