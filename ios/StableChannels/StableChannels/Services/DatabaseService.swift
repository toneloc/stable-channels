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

        try initSchema()
    }

    deinit {
        sqlite3_close(db)
    }

    // MARK: - Schema

    // MARK: - Schema & Migration Initialization

    private let targetSchemaVersion: Int64 = 1

    private func initSchema() throws {
        try configurePragmas()

        let rows = try rawSQL.query("PRAGMA user_version")
        let currentVersion = (rows.first?.first as? Int64) ?? 0

        if currentVersion < targetSchemaVersion {
            try createTablesAndIndexes()
            try applyMigrations()
            try rawSQL.execute("PRAGMA user_version = \(targetSchemaVersion);")
        }

        try pruneHistoricalData()
    }

    private func configurePragmas() throws {
        let pragmas = [
            "PRAGMA journal_mode = WAL;",
            "PRAGMA synchronous = NORMAL;",
            "PRAGMA temp_store = MEMORY;",
            "PRAGMA cache_size = -8000;", // 8 MB page cache (vs default ~2 MB)
            "PRAGMA mmap_size = 134217728;", // 128 MB memory-mapped I/O
            "PRAGMA wal_autocheckpoint = 200;", // checkpoint every 200 pages (vs default 1000)
            "PRAGMA foreign_keys = ON;"
        ]
        for sql in pragmas {
            try rawSQL.execute(sql)
        }
    }

    private func createTablesAndIndexes() throws {
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
            "CREATE INDEX IF NOT EXISTS idx_onchain_receive_txids_status ON onchain_receive_txids(status)",
            "CREATE INDEX IF NOT EXISTS idx_payments_status ON payments(status)",
            "CREATE INDEX IF NOT EXISTS idx_payments_txid ON payments(txid) WHERE txid IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_pending_ops_funding_txid ON pending_operations(funding_outpoint_txid) WHERE funding_outpoint_txid IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_payments_resolution_id ON payments(resolution_id) WHERE resolution_id IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_payments_type_status ON payments(payment_type, status)",
            "CREATE INDEX IF NOT EXISTS idx_trades_channel_id ON trades(channel_id)",
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_payments_payment_id_unique ON payments(payment_id) WHERE payment_id IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_payments_confirmation_scan ON payments(txid, payment_type, status, confirmations) WHERE txid IS NOT NULL"
        ]

        for sql in statements {
            try rawSQL.execute(sql)
        }
    }

    private func applyMigrations() throws {
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

        let paymentsCols = try rawSQL.query("PRAGMA table_info(payments)")
        let paymentsColNames = paymentsCols.compactMap { $0[1] as? String }
        if !paymentsColNames.contains("tx_block_height") {
            try rawSQL.execute("ALTER TABLE payments ADD COLUMN tx_block_height INTEGER")
        }
        if !paymentsColNames.contains("resolution_id") {
            try rawSQL.execute("ALTER TABLE payments ADD COLUMN resolution_id INTEGER")
        }
    }

    private func pruneHistoricalData() throws {
        // Prune price_history rows older than 90 days to prevent unbounded growth
        try rawSQL.execute(
            "DELETE FROM price_history WHERE timestamp < strftime('%s', 'now') - 7776000"
        )
    }
}
