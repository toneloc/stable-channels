import Foundation
import LDKNode
import SQLite3

/// SQLite implementation of PaymentDatabase
final class SQLitePaymentDatabase: PaymentDatabase {
    private let dbPath: String
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
    private static let seenSettlementsTableSQL = """
    CREATE TABLE IF NOT EXISTS seen_stability_settlements (
    settlement_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
    )
    """

    init(dbPath: String) {
        self.dbPath = dbPath
    }

    static var didEnsureColumns = false

    private func openDB(write: Bool = true) -> OpaquePointer? {
        var db: OpaquePointer?
        let flags = write ? SQLITE_OPEN_READWRITE : SQLITE_OPEN_READONLY
        guard sqlite3_open_v2(dbPath, &db, flags, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return nil
        }
        sqlite3_busy_timeout(db, 2000)
        if write && !Self.didEnsureColumns {
            if ensureColumns(db: db) {
                Self.didEnsureColumns = true
            }
        }
        return db
    }

    @discardableResult
    func ensureColumns(db: OpaquePointer?) -> Bool {
        var existingCols = Set<String>()
        var stmt: OpaquePointer?
        if sqlite3_prepare_v2(db, "PRAGMA table_info(payments)", -1, &stmt, nil) == SQLITE_OK {
            while sqlite3_step(stmt) == SQLITE_ROW {
                if let namePtr = sqlite3_column_text(stmt, 1) {
                    existingCols.insert(String(cString: namePtr))
                }
            }
            sqlite3_finalize(stmt)
        } else {
            return false
        }

        // Ensure schema_migrations table exists to track multi-step migrations reliably
        let createMigrationsSQL = """
        CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        """
        if sqlite3_exec(db, createMigrationsSQL, nil, nil, nil) != SQLITE_OK {
            return false
        }

        var success = true
        // 1. is_placeholder column
        if !existingCols.contains("is_placeholder") {
            let alterSQL = "ALTER TABLE payments ADD COLUMN is_placeholder INTEGER NOT NULL DEFAULT 0;"
            var errMsg: UnsafeMutablePointer<CChar>?
            let rc = sqlite3_exec(db, alterSQL, nil, nil, &errMsg)
            if rc != SQLITE_OK {
                if let errPtr = errMsg {
                    let errStr = String(cString: errPtr)
                    sqlite3_free(errMsg)
                    if !errStr.contains("duplicate column name") {
                        success = false
                    }
                } else {
                    success = false
                }
            }
        }

        // 2. backing_applied column and backfill migration with persisted progress
        var isMigrationApplied = false
        var checkStmt: OpaquePointer?
        let checkSQL = "SELECT 1 FROM schema_migrations WHERE name = 'migration_backing_applied_v1' LIMIT 1;"
        if sqlite3_prepare_v2(db, checkSQL, -1, &checkStmt, nil) == SQLITE_OK {
            if sqlite3_step(checkStmt) == SQLITE_ROW {
                isMigrationApplied = true
            }
            sqlite3_finalize(checkStmt)
        }

        if !isMigrationApplied {
            if sqlite3_exec(db, "BEGIN IMMEDIATE;", nil, nil, nil) == SQLITE_OK {
                var migrationFailed = false
                var alreadyMarkedInTransaction = false
                var markerStmt: OpaquePointer?
                if sqlite3_prepare_v2(db, checkSQL, -1, &markerStmt, nil) == SQLITE_OK {
                    alreadyMarkedInTransaction = sqlite3_step(markerStmt) == SQLITE_ROW
                    sqlite3_finalize(markerStmt)
                } else {
                    migrationFailed = true
                }

                var colsNow = Set<String>()
                if !migrationFailed && !alreadyMarkedInTransaction {
                    var colsStmt: OpaquePointer?
                    if sqlite3_prepare_v2(db, "PRAGMA table_info(payments)", -1, &colsStmt, nil) == SQLITE_OK {
                        while sqlite3_step(colsStmt) == SQLITE_ROW {
                            if let namePtr = sqlite3_column_text(colsStmt, 1) {
                                colsNow.insert(String(cString: namePtr))
                            }
                        }
                        sqlite3_finalize(colsStmt)
                    } else {
                        migrationFailed = true
                    }
                }

                if !migrationFailed && !alreadyMarkedInTransaction && !colsNow.contains("backing_applied") {
                    let alterSQL = "ALTER TABLE payments ADD COLUMN backing_applied INTEGER NOT NULL DEFAULT 0;"
                    var errMsg: UnsafeMutablePointer<CChar>?
                    let rc = sqlite3_exec(db, alterSQL, nil, nil, &errMsg)
                    if rc != SQLITE_OK {
                        if let errPtr = errMsg {
                            let errStr = String(cString: errPtr)
                            sqlite3_free(errMsg)
                            if !errStr.contains("duplicate column name") {
                                migrationFailed = true
                            }
                        } else {
                            migrationFailed = true
                        }
                    }

                    if !migrationFailed {
                        // The only unambiguous legacy state is the column-absent upgrade path: all
                        // non-placeholder rows just received backing_applied = 0 from ALTER TABLE, so
                        // mark them applied to prevent old completed stability rows from debiting again.
                        // If the column already exists, a 0 may be a live owed debit and must be left alone.
                        var backfillErr: UnsafeMutablePointer<CChar>?
                        let backfillSQL = "UPDATE payments SET backing_applied = 1 WHERE is_placeholder = 0;"
                        let backfillRc = sqlite3_exec(db, backfillSQL, nil, nil, &backfillErr)
                        if backfillRc != SQLITE_OK {
                            if backfillErr != nil {
                                sqlite3_free(backfillErr)
                            }
                            migrationFailed = true
                        }
                    }
                }

                if !migrationFailed && !alreadyMarkedInTransaction {
                    var markErr: UnsafeMutablePointer<CChar>?
                    let markSQL = "INSERT OR REPLACE INTO schema_migrations (name) VALUES ('migration_backing_applied_v1');"
                    let markRc = sqlite3_exec(db, markSQL, nil, nil, &markErr)
                    if markRc != SQLITE_OK {
                        if markErr != nil {
                            sqlite3_free(markErr)
                        }
                        migrationFailed = true
                    }
                }

                if !migrationFailed {
                    if sqlite3_exec(db, "COMMIT;", nil, nil, nil) != SQLITE_OK {
                        _ = sqlite3_exec(db, "ROLLBACK;", nil, nil, nil)
                        success = false
                    }
                } else {
                    _ = sqlite3_exec(db, "ROLLBACK;", nil, nil, nil)
                    success = false
                }
            } else {
                success = false
            }
        }
        return success
    }

    private func burnSettlement(db: OpaquePointer?, settlementId: String) -> Bool {
        var seenInsert: OpaquePointer?
        let seenSQL = "INSERT INTO seen_stability_settlements (settlement_id) VALUES (?)"
        guard sqlite3_prepare_v2(db, seenSQL, -1, &seenInsert, nil) == SQLITE_OK else { return false }
        bindText(seenInsert, 1, settlementId)
        let burned = sqlite3_step(seenInsert) == SQLITE_DONE
        sqlite3_finalize(seenInsert)
        return burned
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
        userChannelId: String?,
        settlementId: String?
    ) -> PaymentInsertResult {
        guard let db = openDB() else { return .failed }
        defer { sqlite3_close(db) }

        if settlementId != nil {
            _ = sqlite3_exec(db, Self.seenSettlementsTableSQL, nil, nil, nil)
        }

        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return .failed }

        // Replay guard inside the transaction that credits backing, so a crash can never leave
        // backing credited with the settlement id still replayable.
        if let sid = settlementId {
            var seenStmt: OpaquePointer?
            guard sqlite3_prepare_v2(
                db,
                "SELECT 1 FROM seen_stability_settlements WHERE settlement_id = ?",
                -1,
                &seenStmt,
                nil
            ) == SQLITE_OK else {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .failed
            }
            bindText(seenStmt, 1, sid)
            let alreadySeen = sqlite3_step(seenStmt) == SQLITE_ROW
            sqlite3_finalize(seenStmt)
            if alreadySeen {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .duplicate
            }
        }

        // Dedup check
        if !paymentId.isEmpty {
            var checkStmt: OpaquePointer?
            if sqlite3_prepare_v2(
                db,
                "SELECT is_placeholder, backing_applied FROM payments WHERE payment_id = ?",
                -1,
                &checkStmt,
                nil
            ) == SQLITE_OK {
                sqlite3_bind_text(
                    checkStmt,
                    1,
                    (paymentId as NSString).utf8String,
                    -1,
                    SQLITE_TRANSIENT
                )
                if sqlite3_step(checkStmt) == SQLITE_ROW {
                    let isPlaceholder = sqlite3_column_int64(checkStmt, 0) == 1
                    let backingApplied = sqlite3_column_int64(checkStmt, 1) == 1
                    sqlite3_finalize(checkStmt)

                    // Non-crediting early return: seen vs burned divergence is intentional,
                    // matching PaymentRepository semantics so unapplied or duplicate arrivals do not burn settlement
                    // IDs.
                    if !isPlaceholder || backingApplied || backingDeltaSats == nil {
                        sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                        return .duplicate
                    }

                    // Existing row was a placeholder whose backing was never applied.
                    // Apply backing delta now.
                    if let delta = backingDeltaSats {
                        guard let ucid = userChannelId, !ucid.isEmpty else {
                            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                            return .missingChannelRow
                        }
                        if !updateBacking(
                            db: db, ucid: ucid, delta: delta,
                            paymentType: paymentType, direction: direction, btcPrice: btcPrice
                        ) {
                            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                            return .missingChannelRow
                        }
                    }

                    // Burn settlement id if present when crediting backing on placeholder heal.
                    if let sid = settlementId {
                        guard burnSettlement(db: db, settlementId: sid) else {
                            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                            return .failed
                        }
                    }

                    var updateStmt: OpaquePointer?
                    let updateSql = """
                        UPDATE payments
                        SET payment_type = ?,
                            direction = ?,
                            amount_msat = CASE WHEN is_placeholder = 1 OR amount_msat = 0 THEN ? ELSE amount_msat END,
                            amount_usd = COALESCE(amount_usd, ?),
                            btc_price = COALESCE(btc_price, ?),
                            is_placeholder = 0,
                            backing_applied = 1
                        WHERE payment_id = ?
                    """
                    guard sqlite3_prepare_v2(db, updateSql, -1, &updateStmt, nil) == SQLITE_OK else {
                        sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                        return .failed
                    }
                    bindText(updateStmt, 1, paymentType)
                    bindText(updateStmt, 2, direction)
                    sqlite3_bind_int64(updateStmt, 3, Int64(amountMsat))
                    sqlite3_bind_double(updateStmt, 4, amountUSD)
                    sqlite3_bind_double(updateStmt, 5, btcPrice)
                    bindText(updateStmt, 6, paymentId)
                    let stepResult = sqlite3_step(updateStmt)
                    sqlite3_finalize(updateStmt)
                    guard stepResult == SQLITE_DONE else {
                        sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                        return .failed
                    }

                    guard sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else { return .failed }
                    return .inserted
                }
                sqlite3_finalize(checkStmt)
            }
        }

        // Insert payment
        var stmt: OpaquePointer?
        let insertSql = "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status, backing_applied) VALUES (?, ?, ?, ?, ?, ?, 'completed', ?)"
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
        let backingAppliedVal: Int64 = (backingDeltaSats != nil) ? 1 : 0
        sqlite3_bind_int64(stmt, 7, backingAppliedVal)

        guard sqlite3_step(stmt) == SQLITE_DONE else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return .failed
        }

        // Burn the settlement id in the same transaction that credits backing.
        if let sid = settlementId {
            guard burnSettlement(db: db, settlementId: sid) else {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .failed
            }
        }

        // Update backing if needed
        if let delta = backingDeltaSats {
            guard let ucid = userChannelId, !ucid.isEmpty else {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .missingChannelRow
            }
            if !updateBacking(
                db: db, ucid: ucid, delta: delta,
                paymentType: paymentType, direction: direction, btcPrice: btcPrice
            ) {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .missingChannelRow // originally returned missingChannelRow if it fails to update or missing
                // channel
            }
        }

        guard sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else { return .failed }
        return .inserted
    }

    private func updateBacking(
        db: OpaquePointer,
        ucid: String,
        delta: Int64,
        paymentType: String,
        direction: String,
        btcPrice: Double
    ) -> Bool {
        var selectStmt: OpaquePointer?
        guard sqlite3_prepare_v2(
            db,
            "SELECT stable_sats, expected_usd FROM channels WHERE user_channel_id = ?",
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
        let expectedUSD = sqlite3_column_double(selectStmt, 1)
        sqlite3_finalize(selectStmt)

        // Incoming stability credits only the local shortfall (#320); nil rolls the row back.
        guard let newBacking = StabilityAllocation.backingAfterPayment(
            currentBackingSats: currentBacking,
            expectedUSD: expectedUSD,
            price: btcPrice,
            deltaSats: delta,
            paymentType: paymentType,
            direction: direction
        ) else { return false }

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
        SELECT expected_usd, stable_sats, receiver_sats, latest_price, native_sats, user_channel_id, channel_id
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
            userChannelId: sqlite3_column_text(stmt, 5).map { String(cString: $0) } ?? "",
            channelId: sqlite3_column_text(stmt, 6).map { String(cString: $0) } ?? ""
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
        guard let localBacking = TradeProtocol.uncorrelatedSyncBacking(
            receiverSats: receiver,
            currentBackingSats: oldBacking,
            currentExpectedUSD: oldExpected,
            sync: sync,
            price: trustedPrice
        ) else {
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

    // MARK: - Splice Events

    func recordSpliceEvent(_ receipt: SpliceEventReceipt) -> Bool {
        guard let db = openDB() else { return false }
        defer { sqlite3_close(db) }
        for sql in SpliceEventRecorder.createTableStatements
            where sqlite3_exec(db, sql, nil, nil, nil) != SQLITE_OK {
            return false
        }
        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return false }
        let committed = runSpliceStatement(db, SpliceEventRecorder.insertEventSQL, receipt)
            && runSpliceStatement(db, SpliceEventRecorder.attachFundingSQL, receipt)
            && SpliceEventRecorder.assignmentStatements.allSatisfy { runSpliceStatement(db, $0, nil) }
            && sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK
        if !committed {
            rollback(db)
        }
        return committed
    }

    func hasPendingSplice() -> Bool? {
        guard let db = openDB(write: false) else { return nil }
        defer { sqlite3_close(db) }
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, SpliceEventRecorder.hasPendingSpliceSQL, -1, &stmt, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_finalize(stmt) }
        switch sqlite3_step(stmt) {
        case SQLITE_ROW: return true
        case SQLITE_DONE: return false
        default: return nil
        }
    }

    /// Binds the receipt in SpliceEventRecorder parameter order (?1...?6).
    private func runSpliceStatement(_ db: OpaquePointer?, _ sql: String, _ receipt: SpliceEventReceipt?) -> Bool {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
        defer { sqlite3_finalize(stmt) }
        if let receipt {
            let texts = [receipt.eventKey, receipt.kind.rawValue, receipt.userChannelId, receipt.channelId,
                         receipt.fundingTxid]
            for (index, value) in texts.enumerated() {
                if let value {
                    bindText(stmt, Int32(index + 1), value)
                } else {
                    sqlite3_bind_null(stmt, Int32(index + 1))
                }
            }
            if let vout = receipt.fundingVout {
                sqlite3_bind_int64(stmt, 6, Int64(vout))
            } else {
                sqlite3_bind_null(stmt, 6)
            }
        }
        return sqlite3_step(stmt) == SQLITE_DONE
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

        let amountUSD = pending.btcPrice > 0 ? BitcoinUnits
            .usd(msats: pending.amountMsat, price: pending.btcPrice) : 0.0
        let result = recordPayment(
            paymentId: pending.paymentId,
            paymentType: "stability",
            direction: "sent",
            amountMsat: pending.amountMsat,
            amountUSD: amountUSD,
            btcPrice: pending.btcPrice,
            backingDeltaSats: -Int64(pending.amountMsat / 1000),
            userChannelId: activeUserChannelId(),
            settlementId: nil
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

    // MARK: - Outbound Lightning Outcomes (persisted before the LDK event is acked)

    /// A trade fee whose payment id was never attached is adopted by amount, as the foreground
    /// does. No match still acks: the trade then resolves from the LSP's signed result.
    func recordOutgoingPaymentSuccess(
        paymentId: String,
        feeMsat: UInt64?,
        lookupAmountMsat: () -> UInt64? = { nil }
    ) -> Bool {
        guard !paymentId.isEmpty else { return true }
        guard let db = openDB() else { return false }
        defer { sqlite3_close(db) }
        guard let (tradeExists, hasUnattached) = tradeCorrelation(db, paymentId: paymentId) else { return false }
        if !tradeExists, hasUnattached, let amountMsat = lookupAmountMsat() {
            _ = adoptUnattachedPreparedTrade(
                db, paymentId: paymentId, amountMsat: amountMsat,
                sql: SharedPaymentSQL.adoptPaidPaymentForUnattachedTrade
            ) { statement, tradeDbId in
                self.bindText(statement, 1, paymentId)
                self.bindText(statement, 2, paymentId)
                sqlite3_bind_int64(statement, 3, tradeDbId)
            }
        }
        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return false }
        // Fee settlement is not trade acceptance: the trade waits for its signed result.
        let committed = execute(db, SharedPaymentSQL.markTradeFeePaid) { self.bindText($0, 1, paymentId) }
            && execute(db, SharedPaymentSQL.resolvePendingPaymentWithFee) {
                self.bindText($0, 1, "completed")
                sqlite3_bind_int64($0, 2, Int64(clamping: feeMsat ?? 0))
                self.bindText($0, 3, paymentId)
            }
            && sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK
        if !committed { rollback(db) }
        return committed
    }

    func recordOutgoingPaymentFailure(
        paymentId: String,
        reasonCode: String?,
        lookupAmountMsat: () -> UInt64?
    ) -> Bool {
        guard !paymentId.isEmpty else { return true }
        guard let db = openDB() else { return false }
        defer { sqlite3_close(db) }
        guard let (tradeExists, hasUnattached) = tradeCorrelation(db, paymentId: paymentId) else { return false }
        let target = LightningPaymentRecovery.failureTarget(
            paymentId: paymentId,
            pendingStabilityPaymentId: loadPendingSend()?.paymentId,
            tradeExists: tradeExists,
            hasUnattachedPreparedTrade: hasUnattached,
            lookupAmountMsat: lookupAmountMsat
        )
        switch target {
        case .stabilitySend:
            // No sats moved, so there is no debit to record; unblock the next send.
            return execute(db, "DELETE FROM pending_stability_send WHERE id = 1")
        case .trade:
            // Signed results and already-paid fees are protected by the status filter.
            return execute(db, SharedPaymentSQL.markTradeSendFailedByPaymentId) {
                self.bindOptionalText($0, 1, reasonCode)
                self.bindText($0, 2, paymentId)
            }
        case .unattachedTrade(let amountMsat):
            let adopted = adoptUnattachedPreparedTrade(
                db, paymentId: paymentId, amountMsat: amountMsat,
                sql: SharedPaymentSQL.adoptFailedPaymentForUnattachedTrade
            ) { statement, tradeDbId in
                self.bindText(statement, 1, paymentId)
                self.bindText(statement, 2, paymentId)
                self.bindOptionalText(statement, 3, reasonCode)
                sqlite3_bind_int64(statement, 4, tradeDbId)
            }
            if adopted { return true }
            return markPaymentFailed(db, paymentId: paymentId)
        case .payment:
            return markPaymentFailed(db, paymentId: paymentId)
        }
    }

    /// Whether a trade already carries this payment id, and whether any prepared trade still lacks
    /// one; nil when either lookup could not run.
    private func tradeCorrelation(_ db: OpaquePointer?,
                                  paymentId: String) -> (tradeExists: Bool, hasUnattached: Bool)? {
        guard let tradeExists = exists(db, "SELECT 1 FROM trades WHERE trade_payment_id = ? LIMIT 1", paymentId),
              let hasUnattached = exists(
                  db,
                  "SELECT 1 FROM trades WHERE trade_payment_id IS NULL AND status = 'prepared' LIMIT 1",
                  nil
              ) else { return nil }
        return (tradeExists, hasUnattached)
    }

    private func markPaymentFailed(_ db: OpaquePointer?, paymentId: String) -> Bool {
        execute(db, SharedPaymentSQL.resolvePendingPayment) {
            self.bindText($0, 1, "failed")
            self.bindText($0, 2, paymentId)
        }
    }

    /// Same rule and SQL as ChannelRepository's unattached-trade adoption: exactly one recent prepared
    /// trade with this fee amount adopts the payment id through `sql` (bound with its trade row id).
    private func adoptUnattachedPreparedTrade(
        _ db: OpaquePointer?,
        paymentId: String,
        amountMsat: UInt64,
        sql: String,
        bind: (OpaquePointer?, Int64) -> Void
    ) -> Bool {
        let now = UInt64(Date().timeIntervalSince1970)
        guard TradeProtocol.isCanonicalIdentifier(paymentId), amountMsat <= UInt64(Int64.max),
              sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else { return false }
        let cutoff = Int64(max(now, TradeProtocol.responseRetryWindowSecs) - TradeProtocol.responseRetryWindowSecs)
        var select: OpaquePointer?
        let selectSQL = "SELECT id \(SharedPaymentSQL.unattachedPreparedTradeMatch)"
        guard sqlite3_prepare_v2(db, selectSQL, -1, &select, nil) == SQLITE_OK else {
            rollback(db)
            return false
        }
        sqlite3_bind_int64(select, 1, Int64(amountMsat))
        sqlite3_bind_int64(select, 2, cutoff)
        var ids: [Int64] = []
        while sqlite3_step(select) == SQLITE_ROW {
            ids.append(sqlite3_column_int64(select, 0))
        }
        sqlite3_finalize(select)
        guard ids.count == 1, let tradeDbId = ids.first else {
            rollback(db)
            return false
        }
        let updated = execute(db, sql) { bind($0, tradeDbId) } && sqlite3_changes(db) == 1
        guard updated, sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else {
            rollback(db)
            return false
        }
        return true
    }

    /// Runs one statement to completion; false when it could not be prepared or stepped.
    private func execute(
        _ db: OpaquePointer?,
        _ sql: String,
        bind: (OpaquePointer?) -> Void = { _ in }
    ) -> Bool {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { return false }
        defer { sqlite3_finalize(statement) }
        bind(statement)
        return sqlite3_step(statement) == SQLITE_DONE
    }

    /// Whether the query returns a row; nil when it could not run.
    private func exists(_ db: OpaquePointer?, _ sql: String, _ value: String?) -> Bool? {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(statement) }
        if let value { bindText(statement, 1, value) }
        switch sqlite3_step(statement) {
        case SQLITE_ROW: return true
        case SQLITE_DONE: return false
        default: return nil
        }
    }

    private func bindOptionalText(_ statement: OpaquePointer?, _ index: Int32, _ value: String?) {
        if let value { bindText(statement, index, value) } else { sqlite3_bind_null(statement, index) }
    }

    // MARK: - Seen Settlement IDs (STABILITY_PAYMENT_V1 replay protection)

    func isSettlementSeen(settlementId: String) -> Bool {
        guard let db = openDB(write: false) else { return false }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT 1 FROM seen_stability_settlements WHERE settlement_id = ?"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
        defer { sqlite3_finalize(stmt) }

        sqlite3_bind_text(
            stmt,
            1,
            (settlementId as NSString).utf8String,
            -1,
            SQLITE_TRANSIENT
        )
        return sqlite3_step(stmt) == SQLITE_ROW
    }
}
