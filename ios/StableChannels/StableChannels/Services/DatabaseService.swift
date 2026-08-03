//
//  DatabaseService.swift
//  StableChannels
//

import Foundation
import SQLite3

final class DatabaseService {
    static let dbFilename = "stablechannels.db"

    private(set) var db: OpaquePointer?
    let rawSQL: RawSQL

    let channelRepo: ChannelRepository
    let paymentRepo: PaymentRepository
    let spliceRepo: SpliceRepository
    let stabilityRepo: StabilitySendRepository
    let pendingOpRepo: PendingOperationRepository
    let onchainRepo: OnchainReceiveRepository
    let priceRepo: PriceRepository
    let headerRepo: HeaderRepository

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
        self.headerRepo = HeaderRepository(rawSQL: sqlHelper)

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
            "CREATE INDEX IF NOT EXISTS idx_block_headers_hash ON block_headers(hash)",
            "CREATE INDEX IF NOT EXISTS idx_payments_status ON payments(status)",
            "CREATE INDEX IF NOT EXISTS idx_pending_ops_funding_txid ON pending_operations(funding_outpoint_txid) WHERE funding_outpoint_txid IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_payments_type_status ON payments(payment_type, status)",
            "CREATE INDEX IF NOT EXISTS idx_trades_channel_id ON trades(channel_id)",
            "CREATE INDEX IF NOT EXISTS idx_payments_confirmation_scan ON payments(txid, payment_type, status, confirmations) WHERE txid IS NOT NULL"
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

        // Must come after the resolution_id ALTER above — on legacy DBs the column
        // doesn't exist yet when the main statements array runs, and indexing a
        // missing column would abort init.
        try rawSQL.execute(
            "CREATE INDEX IF NOT EXISTS idx_payments_resolution_id ON payments(resolution_id) WHERE resolution_id IS NOT NULL"
        )

        // Unique payment_id index: the engine-level backstop for PaymentRepository's
        // check-then-insert dedup. Existing installs may hold duplicate rows from the
        // pre-NodeDirLock multi-writer era, and building the index over them would
        // fail and abort init — so dedup must run first, keeping the first-recorded
        // row per payment_id (the outcome check-then-insert always intended).
        // Empty-string payment_ids are excluded from the index just like NULLs,
        // matching the `!pid.isEmpty` guard on the app's dedup check.
        let hasUniquePaymentIndex = try !rawSQL.query(
            "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_payments_payment_id_unique'"
        ).isEmpty
        if !hasUniquePaymentIndex {
            try rawSQL.execute("""
                DELETE FROM payments
                WHERE payment_id IS NOT NULL AND payment_id != ''
                  AND id NOT IN (SELECT MIN(id) FROM payments
                                 WHERE payment_id IS NOT NULL AND payment_id != ''
                                 GROUP BY payment_id)
                """)
            try rawSQL.execute("""
                CREATE UNIQUE INDEX IF NOT EXISTS idx_payments_payment_id_unique
                ON payments(payment_id) WHERE payment_id IS NOT NULL AND payment_id != ''
                """)
        }

        try pruneHistoricalData()
    }

    private func pruneHistoricalData() throws {
        let cutoffSeconds = 90 * 86400
        try rawSQL.execute("DELETE FROM price_history WHERE timestamp < strftime('%s', 'now') - \(cutoffSeconds)")
    }

    // MARK: - Transaction & SPV Header Delegations

    func inTransaction(mode: String = "IMMEDIATE", _ block: () throws -> Void) throws {
        try rawSQL.inTransaction(mode: mode, block)
    }

    func fetchLatestHeader() throws -> BlockHeaderRecord? {
        try headerRepo.fetchLatestHeader()
    }

    func storeBlockHeader(_ header: BlockHeaderRecord) throws {
        try headerRepo.insertHeader(
            height: header.height,
            hash: header.hash,
            prevHash: header.prevHash,
            timestamp: header.timestamp
        )
    }

    func rollbackHeadersAbove(height: UInt32) throws {
        try headerRepo.rollbackHeadersAbove(height: height)
    }

    func pruneHeadersOlderThan(height: UInt32) throws {
        try headerRepo.pruneOldHeaders(currentHeight: height)
    }
}
