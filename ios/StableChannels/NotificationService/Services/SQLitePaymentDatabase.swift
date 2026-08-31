import Foundation
import SQLite3
import LDKNode

/// SQLite implementation of PaymentDatabase
final class SQLitePaymentDatabase: PaymentDatabase {
    private let dbPath: String
    private static let satsInBTC: Double = 100_000_000.0
    private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
    private static let pendingSendTableSQL = """
    CREATE TABLE IF NOT EXISTS pending_stability_send (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    payment_id TEXT NOT NULL,
    amount_msat INTEGER NOT NULL,
    price REAL NOT NULL,
    created_at INTEGER NOT NULL
    )
    """

    init(dbPath: String) {
        self.dbPath = dbPath
    }

    private func openDB(write: Bool = true) -> OpaquePointer? {
        var db: OpaquePointer?
        let flags = write ? SQLITE_OPEN_READWRITE : SQLITE_OPEN_READONLY
        guard sqlite3_open_v2(dbPath, &db, flags, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return nil
        }
        sqlite3_busy_timeout(db, 2000)
        return db
    }

    func paymentExists(paymentId: String) -> Bool {
        guard let db = openDB(write: false) else { return false }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT 1 FROM payments WHERE payment_id = ?", -1, &stmt, nil) == SQLITE_OK else {
            return false
        }
        defer { sqlite3_finalize(stmt) }

        sqlite3_bind_text(
            stmt,
            1,
            (paymentId as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )
        return sqlite3_step(stmt) == SQLITE_ROW
    }

    func recordPayment(
        paymentId: String,
        paymentType: String,
        direction: String,
        amountMsat: UInt64,
        amountUSD: Double,
        btcPrice: Double,
        backingDeltaSats: Int64?,
        userChannelId: String?
    ) -> PaymentInsertResult {
        guard let db = openDB() else { return .failed }
        defer { sqlite3_close(db) }

        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return .failed }

        // Dedup check
        if !paymentId.isEmpty {
            var checkStmt: OpaquePointer?
            if sqlite3_prepare_v2(db, "SELECT 1 FROM payments WHERE payment_id = ?", -1, &checkStmt, nil) == SQLITE_OK {
                sqlite3_bind_text(
                    checkStmt,
                    1,
                    (paymentId as NSString).utf8String,
                    -1,
                    SQLITE_TRANSIENT
                )
                if sqlite3_step(checkStmt) == SQLITE_ROW {
                    sqlite3_finalize(checkStmt)
                    sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                    return .duplicate
                }
                sqlite3_finalize(checkStmt)
            }
        }

        // Insert payment
        var stmt: OpaquePointer?
        let insertSql = "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status) VALUES (?, ?, ?, ?, ?, ?, 'completed')"
        guard sqlite3_prepare_v2(db, insertSql, -1, &stmt, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return .failed
        }
        defer { sqlite3_finalize(stmt) }

        if !paymentId.isEmpty {
            sqlite3_bind_text(
                stmt,
                1,
                (paymentId as NSString).utf8String,
                -1,
                SQLITE_TRANSIENT
            )
        } else {
            sqlite3_bind_null(stmt, 1)
        }
        sqlite3_bind_text(
            stmt,
            2,
            (paymentType as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )
        sqlite3_bind_text(
            stmt,
            3,
            (direction as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )
        sqlite3_bind_int64(stmt, 4, Int64(amountMsat))
        sqlite3_bind_double(stmt, 5, amountUSD)
        sqlite3_bind_double(stmt, 6, btcPrice)

        guard sqlite3_step(stmt) == SQLITE_DONE else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return .failed
        }

        // Update backing if needed
        if let delta = backingDeltaSats {
            guard let ucid = userChannelId, !ucid.isEmpty else {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .missingChannelRow
            }
            if !updateBacking(db: db, ucid: ucid, delta: delta) {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .missingChannelRow // originally returned missingChannelRow if it fails to update or missing
                // channel
            }
        }

        guard sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else { return .failed }
        return .inserted
    }

    private func updateBacking(db: OpaquePointer, ucid: String, delta: Int64) -> Bool {
        var selectStmt: OpaquePointer?
        guard sqlite3_prepare_v2(
            db,
            "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
            -1,
            &selectStmt,
            nil
        ) == SQLITE_OK else {
            return false
        }
        sqlite3_bind_text(
            selectStmt,
            1,
            (ucid as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )
        guard sqlite3_step(selectStmt) == SQLITE_ROW else {
            sqlite3_finalize(selectStmt)
            return false
        }
        let currentBacking = sqlite3_column_int64(selectStmt, 0)
        sqlite3_finalize(selectStmt)

        let newBacking = max(0, currentBacking + delta)

        var updateStmt: OpaquePointer?
        let sql = "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s', 'now') WHERE user_channel_id = ?"
        guard sqlite3_prepare_v2(db, sql, -1, &updateStmt, nil) == SQLITE_OK else { return false }
        defer { sqlite3_finalize(updateStmt) }

        sqlite3_bind_int64(updateStmt, 1, newBacking)
        sqlite3_bind_text(
            updateStmt,
            2,
            (ucid as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )

        return sqlite3_step(updateStmt) == SQLITE_DONE && sqlite3_changes(db) == 1
    }

    func readChannelState() -> ChannelState? {
        guard let db = openDB(write: false) else { return nil }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = """
        SELECT expected_usd, stable_sats, receiver_sats, latest_price, native_sats, user_channel_id
        FROM channels
        WHERE user_channel_id IS NOT NULL AND user_channel_id != ''
        ORDER BY updated_at DESC, channel_id DESC
        LIMIT 1
        """
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        guard sqlite3_step(stmt) == SQLITE_ROW else { return nil }

        return ChannelState(
            expectedUSD: sqlite3_column_double(stmt, 0),
            backingSats: UInt64(sqlite3_column_int64(stmt, 1)),
            nativeSats: UInt64(sqlite3_column_int64(stmt, 4)),
            receiverSats: UInt64(sqlite3_column_int64(stmt, 2)),
            latestPrice: sqlite3_column_double(stmt, 3),
            userChannelId: sqlite3_column_text(stmt, 5).map { String(cString: $0) } ?? ""
        )
    }

    func activeUserChannelId() -> String? {
        readChannelState()?.userChannelId
    }

    func applyTradeControl(
        _ message: TradeControlMessage,
        trustedPrice: Double?
    ) -> StableControlDatabaseResult {
        let result: StableControlDatabaseResult = switch message {
        case .rejected(let rejection):
            applyRejection(rejection)
        case .sync(let sync):
            if sync.correlation != nil {
                applyCorrelatedSync(sync)
            } else if let price = trustedPrice, PriceOracle.isPlausibleBitcoinPrice(price) {
                applyUncorrelatedSync(sync, trustedPrice: price)
            } else {
                .retry
            }
        }
        if case .retry = result { markTradeResponseNotCommittable(message) }
        return result
    }

    private func markTradeResponseNotCommittable(_ message: TradeControlMessage) {
        let channelId: String
        let correlation: TradeCorrelation
        switch message {
        case .sync(let sync):
            guard let syncCorrelation = sync.correlation else { return }
            channelId = sync.channelId
            correlation = syncCorrelation
        case .rejected(let rejection):
            channelId = rejection.channelId
            correlation = rejection.correlation
        }
        guard let db = openDB() else { return }
        defer { sqlite3_close(db) }
        var update: OpaquePointer?
        let sql = """
        UPDATE trades SET status = 'uncertain', uncertainty_reason = 'response_not_committable'
        WHERE channel_id = ? AND trade_id = ? AND request_hash = ?
          AND (trade_payment_id = ? OR trade_payment_id IS NULL)
          AND status IN ('prepared','sent','fee_paid','uncertain')
        """
        guard sqlite3_prepare_v2(db, sql, -1, &update, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(update) }
        bindText(update, 1, channelId)
        bindText(update, 2, correlation.tradeId)
        bindText(update, 3, correlation.requestHash)
        bindText(update, 4, correlation.tradePaymentId)
        sqlite3_step(update)
    }

    private func applyCorrelatedSync(
        _ sync: TradeControlMessage.Sync
    ) -> StableControlDatabaseResult {
        guard let correlation = sync.correlation,
              sync.syncVersion <= UInt64(Int64.max) else { return .invalid }
        guard let db = openDB() else { return .retry }
        defer { sqlite3_close(db) }
        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return .retry }

        var tradeStmt: OpaquePointer?
        let tradeSQL = """
        SELECT id, channel_id, user_channel_id, trade_payment_id,
               new_expected_usd, new_backing_sats, status
        FROM trades
        WHERE trade_id = ? AND request_hash = ?
          AND (trade_payment_id = ? OR trade_payment_id IS NULL)
        LIMIT 1
        """
        guard sqlite3_prepare_v2(db, tradeSQL, -1, &tradeStmt, nil) == SQLITE_OK else {
            rollback(db)
            return .retry // Old schema: foreground app must migrate it first.
        }
        bindText(tradeStmt, 1, correlation.tradeId)
        bindText(tradeStmt, 2, correlation.requestHash)
        bindText(tradeStmt, 3, correlation.tradePaymentId)
        guard sqlite3_step(tradeStmt) == SQLITE_ROW else {
            sqlite3_finalize(tradeStmt)
            rollback(db)
            return .invalid
        }
        let tradeRowId = sqlite3_column_int64(tradeStmt, 0)
        let tradeChannel = columnText(tradeStmt, 1)
        let tradeUserChannel = columnText(tradeStmt, 2)
        let storedPayment = sqlite3_column_type(tradeStmt, 3) == SQLITE_NULL ? nil : columnText(tradeStmt, 3)
        let storedExpected = sqlite3_column_double(tradeStmt, 4)
        let storedBackingSigned = sqlite3_column_int64(tradeStmt, 5)
        let status = columnText(tradeStmt, 6)
        sqlite3_finalize(tradeStmt)

        guard tradeChannel == sync.channelId, tradeUserChannel == sync.userChannelId,
              storedPayment == nil || storedPayment == correlation.tradePaymentId,
              abs(storedExpected - sync.expectedUSD) <= 0.000000001,
              storedBackingSigned >= 0 else {
            rollback(db)
            return .invalid
        }
        if status == "accepted" {
            rollback(db)
            return .duplicate
        }
        guard status != "rejected", status != "send_failed" else {
            rollback(db)
            return .invalid
        }

        var channelStmt: OpaquePointer?
        guard sqlite3_prepare_v2(
            db,
            "SELECT channel_id, receiver_sats, sync_version, stable_sats FROM channels WHERE user_channel_id = ?",
            -1,
            &channelStmt,
            nil
        ) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        bindText(channelStmt, 1, sync.userChannelId)
        guard sqlite3_step(channelStmt) == SQLITE_ROW else {
            sqlite3_finalize(channelStmt)
            rollback(db)
            return .retry
        }
        let channelId = columnText(channelStmt, 0)
        let receiverSigned = sqlite3_column_int64(channelStmt, 1)
        let currentVersion = sqlite3_column_int64(channelStmt, 2)
        let currentBackingSigned = sqlite3_column_int64(channelStmt, 3)
        sqlite3_finalize(channelStmt)
        guard channelId == sync.channelId else {
            rollback(db)
            return .invalid
        }
        guard receiverSigned >= 0, currentBackingSigned >= 0 else {
            rollback(db)
            return .retry
        }
        if Int64(sync.syncVersion) > currentVersion {
            guard storedBackingSigned <= receiverSigned else {
                rollback(db)
                return .retry
            }
            let nativeSigned = receiverSigned - storedBackingSigned
            var updateChannel: OpaquePointer?
            let updateChannelSQL = """
            UPDATE channels
            SET expected_usd = ?, stable_sats = ?, native_sats = ?, sync_version = ?,
                updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ? AND channel_id = ? AND sync_version < ?
            """
            guard sqlite3_prepare_v2(db, updateChannelSQL, -1, &updateChannel, nil) == SQLITE_OK else {
                rollback(db)
                return .retry
            }
            sqlite3_bind_double(updateChannel, 1, sync.expectedUSD)
            sqlite3_bind_int64(updateChannel, 2, storedBackingSigned)
            sqlite3_bind_int64(updateChannel, 3, nativeSigned)
            sqlite3_bind_int64(updateChannel, 4, Int64(sync.syncVersion))
            bindText(updateChannel, 5, sync.userChannelId)
            bindText(updateChannel, 6, sync.channelId)
            sqlite3_bind_int64(updateChannel, 7, Int64(sync.syncVersion))
            let channelUpdated = sqlite3_step(updateChannel) == SQLITE_DONE && sqlite3_changes(db) == 1
            sqlite3_finalize(updateChannel)
            guard channelUpdated else {
                rollback(db)
                return .retry
            }
        }

        var updateTrade: OpaquePointer?
        let updateTradeSQL = """
        UPDATE trades
        SET payment_id = ?, trade_payment_id = ?, status = 'accepted', outcome = 'accepted',
            resolved_at = strftime('%s', 'now'), uncertainty_reason = NULL
        WHERE id = ?
        """
        guard sqlite3_prepare_v2(db, updateTradeSQL, -1, &updateTrade, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        bindText(updateTrade, 1, correlation.tradePaymentId)
        bindText(updateTrade, 2, correlation.tradePaymentId)
        sqlite3_bind_int64(updateTrade, 3, tradeRowId)
        let tradeUpdated = sqlite3_step(updateTrade) == SQLITE_DONE && sqlite3_changes(db) == 1
        sqlite3_finalize(updateTrade)
        guard tradeUpdated, sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        return .applied
    }

    private func applyRejection(
        _ rejection: TradeControlMessage.Rejected
    ) -> StableControlDatabaseResult {
        guard rejection.decidedAt <= UInt64(Int64.max) else { return .invalid }
        guard let db = openDB() else { return .retry }
        defer { sqlite3_close(db) }
        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return .retry }

        var select: OpaquePointer?
        let sql = """
        SELECT id, channel_id, trade_payment_id, status
        FROM trades
        WHERE trade_id = ? AND request_hash = ?
          AND (trade_payment_id = ? OR trade_payment_id IS NULL)
        LIMIT 1
        """
        guard sqlite3_prepare_v2(db, sql, -1, &select, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        bindText(select, 1, rejection.correlation.tradeId)
        bindText(select, 2, rejection.correlation.requestHash)
        bindText(select, 3, rejection.correlation.tradePaymentId)
        guard sqlite3_step(select) == SQLITE_ROW else {
            sqlite3_finalize(select)
            rollback(db)
            return .invalid
        }
        let rowId = sqlite3_column_int64(select, 0)
        let channelId = columnText(select, 1)
        let paymentId = sqlite3_column_type(select, 2) == SQLITE_NULL ? nil : columnText(select, 2)
        let status = columnText(select, 3)
        sqlite3_finalize(select)
        guard channelId == rejection.channelId,
              paymentId == nil || paymentId == rejection.correlation.tradePaymentId else {
            rollback(db)
            return .invalid
        }
        if status == "rejected" {
            rollback(db)
            return .duplicate
        }
        guard status != "accepted", status != "send_failed" else {
            rollback(db)
            return .invalid
        }

        var update: OpaquePointer?
        let updateSQL = """
        UPDATE trades
        SET payment_id = ?, trade_payment_id = ?, status = 'rejected', outcome = 'rejected',
            reason_code = ?, resolved_at = ?, uncertainty_reason = NULL
        WHERE id = ?
        """
        guard sqlite3_prepare_v2(db, updateSQL, -1, &update, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        bindText(update, 1, rejection.correlation.tradePaymentId)
        bindText(update, 2, rejection.correlation.tradePaymentId)
        bindText(update, 3, rejection.reasonCode)
        sqlite3_bind_int64(update, 4, Int64(rejection.decidedAt))
        sqlite3_bind_int64(update, 5, rowId)
        let updated = sqlite3_step(update) == SQLITE_DONE && sqlite3_changes(db) == 1
        sqlite3_finalize(update)
        guard updated, sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        return .applied
    }

    private func applyUncorrelatedSync(
        _ sync: TradeControlMessage.Sync,
        trustedPrice: Double
    ) -> StableControlDatabaseResult {
        guard sync.correlation == nil, sync.syncVersion <= UInt64(Int64.max) else { return .invalid }
        guard let db = openDB() else { return .retry }
        defer { sqlite3_close(db) }
        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return .retry }

        var select: OpaquePointer?
        let selectSQL = """
        SELECT channel_id, expected_usd, stable_sats, receiver_sats, sync_version
        FROM channels WHERE user_channel_id = ?
        """
        guard sqlite3_prepare_v2(db, selectSQL, -1, &select, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        bindText(select, 1, sync.userChannelId)
        guard sqlite3_step(select) == SQLITE_ROW else {
            sqlite3_finalize(select)
            rollback(db)
            return .retry
        }
        let channelId = columnText(select, 0)
        let oldExpected = sqlite3_column_double(select, 1)
        let oldBackingSigned = sqlite3_column_int64(select, 2)
        let receiverSigned = sqlite3_column_int64(select, 3)
        let oldVersion = sqlite3_column_int64(select, 4)
        sqlite3_finalize(select)
        guard channelId == sync.channelId else {
            rollback(db)
            return .invalid
        }
        if Int64(sync.syncVersion) <= oldVersion {
            rollback(db)
            return .duplicate
        }
        guard oldBackingSigned >= 0, receiverSigned >= 0 else {
            rollback(db)
            return .retry
        }
        let oldBacking = UInt64(oldBackingSigned)
        let receiver = UInt64(receiverSigned)
        let localBacking: UInt64
        if sync.expectedUSD == 0 {
            localBacking = 0
        } else if oldBacking > 0, sync.expectedUSD == oldExpected {
            localBacking = min(oldBacking, receiver)
        } else if let calculated = TradeProtocol.tradeBackingAfterDelta(
            receiverSats: receiver,
            currentBackingSats: oldBacking,
            currentExpectedUSD: oldExpected,
            newExpectedUSD: sync.expectedUSD,
            price: trustedPrice
        ) {
            localBacking = calculated
        } else {
            rollback(db)
            return .retry
        }
        let native = receiver - localBacking

        var update: OpaquePointer?
        let updateSQL = """
        UPDATE channels
        SET expected_usd = ?, stable_sats = ?, native_sats = ?, sync_version = ?,
            latest_price = ?, updated_at = strftime('%s', 'now')
        WHERE user_channel_id = ? AND channel_id = ? AND sync_version < ?
        """
        guard sqlite3_prepare_v2(db, updateSQL, -1, &update, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        sqlite3_bind_double(update, 1, sync.expectedUSD)
        sqlite3_bind_int64(update, 2, Int64(localBacking))
        sqlite3_bind_int64(update, 3, Int64(native))
        sqlite3_bind_int64(update, 4, Int64(sync.syncVersion))
        sqlite3_bind_double(update, 5, trustedPrice)
        bindText(update, 6, sync.userChannelId)
        bindText(update, 7, sync.channelId)
        sqlite3_bind_int64(update, 8, Int64(sync.syncVersion))
        let updated = sqlite3_step(update) == SQLITE_DONE && sqlite3_changes(db) == 1
        sqlite3_finalize(update)
        guard updated, sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else {
            rollback(db)
            return .retry
        }
        return .applied
    }

    private func bindText(_ statement: OpaquePointer?, _ index: Int32, _ value: String) {
        sqlite3_bind_text(statement, index, (value as NSString).utf8String, -1, SQLITE_TRANSIENT)
    }

    private func columnText(_ statement: OpaquePointer?, _ index: Int32) -> String {
        sqlite3_column_text(statement, index).map { String(cString: $0) } ?? ""
    }

    private func rollback(_ db: OpaquePointer?) {
        sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
    }

    // MARK: - Pending Send Operations

    func claimPendingSend(amountMsat: UInt64, price: Double) -> Bool {
        guard let db = openDB() else { return false }
        defer { sqlite3_close(db) }

        _ = sqlite3_exec(db, Self.pendingSendTableSQL, nil, nil, nil)

        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return false }

        var checkStmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT 1 FROM pending_stability_send WHERE id = 1", -1, &checkStmt, nil) ==
            SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }
        let alreadyClaimed = sqlite3_step(checkStmt) == SQLITE_ROW
        sqlite3_finalize(checkStmt)

        if alreadyClaimed {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }

        var stmt: OpaquePointer?
        let sql = "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }
        defer { sqlite3_finalize(stmt) }

        sqlite3_bind_int64(stmt, 1, Int64(amountMsat))
        sqlite3_bind_double(stmt, 2, price)
        sqlite3_bind_int64(stmt, 3, Int64(Date().timeIntervalSince1970))

        guard sqlite3_step(stmt) == SQLITE_DONE else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }

        return sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK
    }

    func loadPendingSend() -> PendingOutgoingStabilityPayment? {
        guard let db = openDB(write: false) else { return nil }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        guard sqlite3_step(stmt) == SQLITE_ROW else { return nil }

        return PendingOutgoingStabilityPayment(
            paymentId: sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? "",
            amountMsat: UInt64(sqlite3_column_int64(stmt, 1)),
            btcPrice: sqlite3_column_double(stmt, 2),
            createdAt: sqlite3_column_int64(stmt, 3)
        )
    }

    func clearPendingSend() {
        guard let db = openDB() else { return }
        sqlite3_exec(db, "DELETE FROM pending_stability_send WHERE id = 1", nil, nil, nil)
        sqlite3_close(db)
    }

    func reconcilePendingOutgoingPayment(node: LDKNode.Node) -> Bool {
        guard var pending = loadPendingSend() else { return true }

        if pending.paymentId.isEmpty {
            let candidates = node.listPayments().filter { payment in
                guard payment.direction == .outbound,
                      payment.amountMsat == pending.amountMsat,
                      Int64(payment.latestUpdateTimestamp) >= pending.createdAt - 10,
                      case .spontaneous = payment.kind else { return false }
                return true
            }

            if let succeeded = candidates.first(where: { $0.status == .succeeded }) {
                _ = setPendingSendPaymentId(paymentId: "\(succeeded.id)")
                pending = PendingOutgoingStabilityPayment(
                    paymentId: "\(succeeded.id)",
                    amountMsat: pending.amountMsat,
                    btcPrice: pending.btcPrice,
                    createdAt: pending.createdAt
                )
            } else if candidates.contains(where: { $0.status == .pending }) {
                return false
            } else if candidates
                .contains(where: { $0.status == .failed }) || Int64(Date().timeIntervalSince1970) - pending
                .createdAt > 120 {
                clearPendingSend()
                return true
            } else {
                return false
            }
        }

        let amountUSD = pending.btcPrice > 0 ? (Double(pending.amountMsat) / 1000.0 / Self.satsInBTC) * pending
            .btcPrice : 0.0
        let result = recordPayment(
            paymentId: pending.paymentId,
            paymentType: "stability",
            direction: "sent",
            amountMsat: pending.amountMsat,
            amountUSD: amountUSD,
            btcPrice: pending.btcPrice,
            backingDeltaSats: -Int64(pending.amountMsat / 1000),
            userChannelId: activeUserChannelId()
        )

        switch result {
        case .inserted, .duplicate:
            clearPendingSend()
            return true
        case .failed, .missingChannelRow:
            return false
        }
    }

    func setPendingSendPaymentId(paymentId: String) -> Bool {
        guard let db = openDB() else { return false }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "UPDATE pending_stability_send SET payment_id = ? WHERE id = 1", -1, &stmt, nil) ==
            SQLITE_OK else {
            return false
        }
        defer { sqlite3_finalize(stmt) }

        sqlite3_bind_text(
            stmt,
            1,
            (paymentId as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )
        return sqlite3_step(stmt) == SQLITE_DONE
    }
}
