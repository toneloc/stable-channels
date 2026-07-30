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
        self.onchainRepo = OnchainReceiveRepository(rawSQL: sqlHelper)
        self.priceRepo = PriceRepository(rawSQL: sqlHelper)

        try initSchema()
    }

    deinit {
        sqlite3_close(db)
    }

    // MARK: - Schema

    private func initSchema() throws {
        let statements = [
            "PRAGMA journal_mode = WAL;",
            "PRAGMA synchronous = NORMAL;",
            "PRAGMA temp_store = MEMORY;",
            "PRAGMA cache_size = -8000;", // 8 MB page cache (vs default ~2 MB)
            "PRAGMA mmap_size = 134217728;", // 128 MB memory-mapped I/O
            "PRAGMA wal_autocheckpoint = 200;", // checkpoint every 200 pages (vs default 1000)
            "PRAGMA foreign_keys = ON;",
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
            "CREATE INDEX IF NOT EXISTS idx_payments_payment_id ON payments(payment_id)",
            "CREATE INDEX IF NOT EXISTS idx_payments_status ON payments(status)",
            "CREATE INDEX IF NOT EXISTS idx_payments_txid ON payments(txid) WHERE txid IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_pending_ops_funding_txid ON pending_operations(funding_outpoint_txid) WHERE funding_outpoint_txid IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_payments_resolution_id ON payments(resolution_id) WHERE resolution_id IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_payments_type_status ON payments(payment_type, status)",
            "CREATE INDEX IF NOT EXISTS idx_trades_channel_id ON trades(channel_id)"
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

        // Prune price_history rows older than 90 days to prevent unbounded growth
        try rawSQL.execute(
            "DELETE FROM price_history WHERE timestamp < strftime('%s', 'now') - 7776000"
        )
    }

    // MARK: - Channel Operations Facade

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
        try channelRepo.saveChannel(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: expectedUSD,
            backingSats: backingSats,
            nativeSats: nativeSats,
            note: note,
            receiverSats: receiverSats,
            latestPrice: latestPrice
        )
    }

    func saveChannelPreservingBacking(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        nativeSats: UInt64 = 0,
        note: String?,
        receiverSats: UInt64 = 0,
        latestPrice: Double = 0.0
    ) throws {
        try channelRepo.saveChannelPreservingBacking(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: expectedUSD,
            nativeSats: nativeSats,
            note: note,
            receiverSats: receiverSats,
            latestPrice: latestPrice
        )
    }

    func loadChannel(userChannelId: String? = nil) throws -> ChannelRecord? {
        try channelRepo.loadChannel(userChannelId: userChannelId)
    }

    func deleteChannel(userChannelId: String) throws {
        try channelRepo.deleteChannel(userChannelId: userChannelId)
    }

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
        try channelRepo.recordTrade(
            channelId: channelId,
            action: action,
            amountUSD: amountUSD,
            amountBTC: amountBTC,
            btcPrice: btcPrice,
            feeUSD: feeUSD,
            paymentId: paymentId,
            status: status
        )
    }

    func getRecentTrades(limit: Int) throws -> [TradeRecord] {
        try channelRepo.getRecentTrades(limit: limit)
    }

    // MARK: - Payment Operations Facade

    func paymentExists(txid: String, excludePaymentId: String) -> Bool {
        paymentRepo.paymentExists(txid: txid, excludePaymentId: excludePaymentId)
    }

    func deletePayment(paymentId: String) {
        paymentRepo.deletePayment(paymentId: paymentId)
    }

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
        try paymentRepo.recordPayment(
            paymentId: paymentId,
            paymentType: paymentType,
            direction: direction,
            amountMsat: amountMsat,
            amountUSD: amountUSD,
            btcPrice: btcPrice,
            counterparty: counterparty,
            status: status,
            txid: txid,
            address: address
        )
    }

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
        try paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: paymentId,
            paymentType: paymentType,
            direction: direction,
            amountMsat: amountMsat,
            amountUSD: amountUSD,
            btcPrice: btcPrice,
            status: status,
            userChannelId: userChannelId,
            backingDeltaSats: backingDeltaSats
        )
    }

    func claimPendingSend(amountMsat: UInt64, price: Double) -> Bool {
        paymentRepo.claimPendingSend(amountMsat: amountMsat, price: price)
    }

    @discardableResult
    func setPendingSendPaymentId(_ paymentId: String) -> Bool {
        paymentRepo.setPendingSendPaymentId(paymentId)
    }

    func loadPendingSend() -> PendingStabilitySend? {
        paymentRepo.loadPendingSend()
    }

    func clearPendingSend() {
        paymentRepo.clearPendingSend()
    }

    func latestReceivedPayment() -> PaymentRecord? {
        paymentRepo.latestReceivedPayment()
    }

    func payment(paymentId: String) -> PaymentRecord? {
        paymentRepo.payment(paymentId: paymentId)
    }

    func paymentsNeedingConfirmation() throws -> [PaymentRecord] {
        try paymentRepo.paymentsNeedingConfirmation()
    }

    func getPayment(byId id: Int64) throws -> PaymentRecord? {
        try paymentRepo.getPayment(byId: id)
    }

    func updateConfirmations(paymentId: Int64, txBlockHeight: UInt32, currentBlockHeight: UInt32) throws {
        try paymentRepo.updateConfirmations(
            paymentId: paymentId,
            txBlockHeight: txBlockHeight,
            currentBlockHeight: currentBlockHeight
        )
    }

    func getRecentPayments(limit: Int) throws -> [PaymentRecord] {
        try paymentRepo.getRecentPayments(limit: limit)
    }

    func updateTradeStatus(_ tradeId: Int64, status: String) throws {
        try paymentRepo.updateTradeStatus(tradeId, status: status)
    }

    func setPendingSpliceTxid(_ txid: String) throws {
        try paymentRepo.setPendingSpliceTxid(txid)
    }

    func getPendingSpliceTxid() throws -> String? {
        try paymentRepo.getPendingSpliceTxid()
    }

    func hasPendingSplice() throws -> Bool {
        try paymentRepo.hasPendingSplice()
    }

    @discardableResult
    func completeLatestSplice(txid: String?) -> Bool {
        paymentRepo.completeLatestSplice(txid: txid)
    }

    @discardableResult
    func completeSplice(txid: String) -> Bool {
        paymentRepo.completeSplice(txid: txid)
    }

    @discardableResult
    func failLatestPendingSplice() -> Bool {
        paymentRepo.failLatestPendingSplice()
    }

    func updatePaymentStatus(paymentId: String, status: String, feeMsat: UInt64? = nil) throws {
        try paymentRepo.updatePaymentStatus(paymentId: paymentId, status: status, feeMsat: feeMsat)
    }

    func failPaymentByTxid(txid: String) throws {
        try paymentRepo.failPaymentByTxid(txid: txid)
    }

    func isOutgoingStabilityPayment(paymentId: String) throws -> Bool {
        try paymentRepo.isOutgoingStabilityPayment(paymentId: paymentId)
    }

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
        paymentRepo.insertPendingOperation(
            opId: opId,
            opType: opType,
            fundingOutpointTxid: fundingOutpointTxid,
            fundingOutpointVout: fundingOutpointVout,
            balanceSats: balanceSats,
            balanceUsd: balanceUsd,
            btcPrice: btcPrice,
            counterparty: counterparty
        )
    }

    @discardableResult
    func updatePendingOperation(opId: String, closingTxid: String, status: String) -> Bool {
        paymentRepo.updatePendingOperation(opId: opId, closingTxid: closingTxid, status: status)
    }

    func fetchPendingOperations() -> [PendingOperation] {
        paymentRepo.fetchPendingOperations()
    }

    func fetchPendingOperation(opId: String) -> PendingOperation? {
        paymentRepo.fetchPendingOperation(opId: opId)
    }

    func fetchPendingOperationByFundingTxid(_ txid: String) -> PendingOperation? {
        paymentRepo.fetchPendingOperationByFundingTxid(txid)
    }

    // MARK: - Onchain Receive Facade

    @discardableResult
    func insertOnchainReceiveResolution(address: String) -> Int64? {
        onchainRepo.insertOnchainReceiveResolution(address: address)
    }

    func fetchPendingOnchainReceives() -> [OnchainReceiveResolution] {
        onchainRepo.fetchPendingOnchainReceives()
    }

    @discardableResult
    func updateOnchainReceiveResolution(id: Int64, txid: String) -> Bool {
        onchainRepo.updateOnchainReceiveResolution(id: id, txid: txid)
    }

    func fetchPendingOnchainReceiveRows() -> [PendingOnchainPayment] {
        onchainRepo.fetchPendingOnchainReceiveRows()
    }

    @discardableResult
    func updatePaymentTxid(paymentId: String, txid: String, status: String) -> Bool {
        paymentRepo.updatePaymentTxid(paymentId: paymentId, txid: txid, status: status)
    }

    @discardableResult
    func updatePaymentResolution(paymentId: String, resolutionId: Int64) -> Bool {
        onchainRepo.updatePaymentResolution(paymentId: paymentId, resolutionId: resolutionId)
    }

    @discardableResult
    func recordOnchainPaymentWithResolution(
        paymentId: String,
        amountMsat: Int64,
        amountUSD: Double?,
        btcPrice: Double?,
        resolutionId: Int64
    ) -> Bool {
        onchainRepo.recordOnchainPaymentWithResolution(
            paymentId: paymentId,
            amountMsat: amountMsat,
            amountUSD: amountUSD,
            btcPrice: btcPrice,
            resolutionId: resolutionId
        )
    }

    func fetchPendingOnchainReceiveRow(resolutionId: Int64) -> PendingOnchainPayment? {
        onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resolutionId)
    }

    func fetchLatestResolvedOnchainTxid() -> String? {
        onchainRepo.fetchLatestResolvedOnchainTxid()
    }

    @discardableResult
    func deleteOnchainReceiveResolution(id: Int64) -> Bool {
        onchainRepo.deleteOnchainReceiveResolution(id: id)
    }

    // MARK: - Price History Facade

    func recordPrice(_ price: Double, source: String? = nil) throws {
        try priceRepo.recordPrice(price, source: source)
    }

    func backfillHourlyPrices(_ prices: [(timestamp: Int64, price: Double)]) throws -> Int {
        try priceRepo.backfillHourlyPrices(prices)
    }

    func getOldestPriceHistoryTimestamp() throws -> Int64? {
        try priceRepo.getOldestPriceHistoryTimestamp()
    }

    func getPriceHistory(hours: UInt32) throws -> [PriceRecord] {
        try priceRepo.getPriceHistory(hours: hours)
    }

    func getDailyPrices(days: UInt32) throws -> [DailyPriceRecord] {
        try priceRepo.getDailyPrices(days: days)
    }

    func recordDailyPrice(
        date: String, open: Double, high: Double, low: Double,
        close: Double, volume: Double?, source: String? = nil
    ) throws {
        try priceRepo.recordDailyPrice(
            date: date, open: open, high: high, low: low,
            close: close, volume: volume, source: source
        )
    }

    func bulkInsertDailyPrices(_ prices: [(String, Double, Double, Double, Double, Double?)]) throws -> Int {
        try priceRepo.bulkInsertDailyPrices(prices)
    }

    func getOldestDailyPriceDate() throws -> String? {
        try priceRepo.getOldestDailyPriceDate()
    }
}
