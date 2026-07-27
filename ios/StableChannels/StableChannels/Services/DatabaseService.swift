import Foundation
import SQLite3

/// SQLite database layer — port of src/db.rs
/// Uses raw SQLite3 C API to avoid external dependencies initially.
class DatabaseService {
    private var db: OpaquePointer?

    static let dbFilename = "stablechannels.db"

    let rawSQL: RawSQL
    let channelRepo: ChannelRepository
    let paymentRepo: PaymentRepository
    let spliceRepo: SpliceRepository
    let stabilityRepo: StabilitySendRepository
    let pendingOpRepo: PendingOperationRepository
    let onchainRepo: OnchainReceiveRepository
    let priceRepo: PriceRepository

    init(dataDir: URL) throws {
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        let dbPath = dataDir.appendingPathComponent(Self.dbFilename).path

        var databaseHandle: OpaquePointer?
        guard sqlite3_open(dbPath, &databaseHandle) == SQLITE_OK else {
            throw DatabaseError.openFailed(String(cString: sqlite3_errmsg(databaseHandle)))
        }
        self.db = databaseHandle

        // Main app and NSE genuinely overlap on this DB — wait briefly for locks
        // instead of failing instantly with SQLITE_BUSY.
        sqlite3_busy_timeout(db, 2000)

        let sqlHelper = RawSQL { databaseHandle }
        self.rawSQL = sqlHelper

        self.channelRepo = ChannelRepository(rawSQL: sqlHelper)
        self.paymentRepo = PaymentRepository(rawSQL: sqlHelper)
        self.spliceRepo = SpliceRepository(rawSQL: sqlHelper)
        self.stabilityRepo = StabilitySendRepository(rawSQL: sqlHelper)
        self.pendingOpRepo = PendingOperationRepository(rawSQL: sqlHelper)
        self.onchainRepo = OnchainReceiveRepository(rawSQL: sqlHelper)
        self.priceRepo = PriceRepository(rawSQL: sqlHelper)

        sqlHelper.getDB = { [weak self] in self?.db }

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
            """
            CREATE TABLE IF NOT EXISTS block_headers (
                height INTEGER PRIMARY KEY,
                hash TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )
            """,
            "CREATE INDEX IF NOT EXISTS idx_price_history_timestamp ON price_history(timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_pending_operations_status ON pending_operations(status)",
            "CREATE INDEX IF NOT EXISTS idx_payments_created ON payments(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_daily_prices_date ON daily_prices(date DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_txs_created ON onchain_txs(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_receive_txids_status ON onchain_receive_txids(status)",
            "CREATE INDEX IF NOT EXISTS idx_block_headers_hash ON block_headers(hash)"
        ]

        for sql in statements {
            try rawSQL.execute(sql)
        }

        // Migrate: add receiver_sats and latest_price if missing
        let cols = try rawSQL.query("PRAGMA table_info(channels)")
        let colNames = cols.compactMap { $0[1] as? String }
        if !colNames.contains("receiver_sats") {
            try rawSQL.execute("ALTER TABLE channels ADD COLUMN receiver_sats INTEGER NOT NULL DEFAULT 0")
        }
        if !colNames.contains("latest_price") {
            try rawSQL.execute("ALTER TABLE channels ADD COLUMN latest_price REAL NOT NULL DEFAULT 0.0")
        }
        if !colNames.contains("native_sats") {
            try rawSQL.execute("ALTER TABLE channels ADD COLUMN native_sats INTEGER NOT NULL DEFAULT 0")
        }

        // Migrate: add tx_block_height to payments if missing (on-chain confirmation tracking)
        let paymentsCols = try rawSQL.query("PRAGMA table_info(payments)")
        let paymentsColNames = paymentsCols.compactMap { $0[1] as? String }
        if !paymentsColNames.contains("tx_block_height") {
            try rawSQL.execute("ALTER TABLE payments ADD COLUMN tx_block_height INTEGER")
        }

        // Migrate: add resolution_id to payments if missing (onchain deposit <-> resolver link)
        if !paymentsColNames.contains("resolution_id") {
            try rawSQL.execute("ALTER TABLE payments ADD COLUMN resolution_id INTEGER")
        }
    }
}
// MARK: - Block Headers (SPV)

extension DatabaseService {
    /// Maximum number of block headers to retain in the local chain.
    /// 2016 = one Bitcoin difficulty epoch (~2 weeks): wide enough to cover any realistic
    /// reorg depth while keeping the table under ~160 KB on disk.
    static let headerRetentionDepth: UInt32 = 2016

    func insertHeader(height: UInt32, hash: String, prevHash: String, timestamp: UInt32) throws {
        let sql = """
        INSERT OR REPLACE INTO block_headers (height, hash, prev_hash, timestamp)
        VALUES (?, ?, ?, ?);
        """
        try rawSQL.execute(sql, params: [
            .integer(Int64(height)),
            .text(hash),
            .text(prevHash),
            .integer(Int64(timestamp))
        ])
    }

    /// Returns true if we already have a header for this height+hash (idempotent guard).
    func headerExists(height: UInt32, hash: String) throws -> Bool {
        let sql = "SELECT 1 FROM block_headers WHERE height = ? AND hash = ? LIMIT 1;"
        let rows = try query(sql, params: [.integer(Int64(height)), .text(hash)])
        return !rows.isEmpty
    }

    func fetchLatestHeader() throws -> BlockHeaderRecord? {
        let sql = "SELECT height, hash, prev_hash, timestamp FROM block_headers ORDER BY height DESC LIMIT 1;"
        let rows = try rawSQL.query(sql)
        guard let row = rows.first,
              let rawHeight = row[0] as? Int64,
              let hash = row[1] as? String,
              let prevHash = row[2] as? String,
              let rawTimestamp = row[3] as? Int64,
              let height = UInt32(exactly: rawHeight),
              let timestamp = UInt32(exactly: rawTimestamp) else {
            return nil
        }
        return BlockHeaderRecord(
            height: height,
            hash: hash,
            prevHash: prevHash,
            timestamp: timestamp
        )
    }

    func findCommonAncestorHeight(prevHash: String) throws -> UInt32? {
        let sql = "SELECT height FROM block_headers WHERE hash = ? LIMIT 1;"
        let rows = try rawSQL.query(sql, params: [.text(prevHash)])
        guard let row = rows.first,
              let rawHeight = row[0] as? Int64,
              let height = UInt32(exactly: rawHeight) else {
            return nil
        }
        return height
    }

    func rollbackHeadersAbove(height: UInt32) throws {
        let sql = "DELETE FROM block_headers WHERE height > ?;"
        try rawSQL.execute(sql, params: [.integer(Int64(height))])
    }

    func rollbackPaymentsConfirmedAfter(height: UInt32) throws {
        let sql = """
        UPDATE payments
        SET status = 'pending', confirmations = 0
        WHERE confirmations > 0 AND created_at > ?;
        """
        try rawSQL.execute(sql, params: [.integer(Int64(height))])
    }

    /// Rolling-window prune that keeps only the most recent `headerRetentionDepth` headers.
    /// Older entries are silently discarded — they predate any realistic reorg depth.
    func pruneOldHeaders(currentHeight: UInt32) throws {
        guard currentHeight >= Self.headerRetentionDepth else { return }
        let cutoff = currentHeight - Self.headerRetentionDepth
        let sql = "DELETE FROM block_headers WHERE height < ?;"
        try execute(sql, params: [.integer(Int64(cutoff))])
    }
}
