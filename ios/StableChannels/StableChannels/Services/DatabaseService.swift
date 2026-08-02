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
            "CREATE INDEX IF NOT EXISTS idx_price_history_timestamp ON price_history(timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_pending_operations_status ON pending_operations(status)",
            "CREATE INDEX IF NOT EXISTS idx_payments_created ON payments(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_daily_prices_date ON daily_prices(date DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_txs_created ON onchain_txs(created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_onchain_receive_txids_status ON onchain_receive_txids(status)"
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
