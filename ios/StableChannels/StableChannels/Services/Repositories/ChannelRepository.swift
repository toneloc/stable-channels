import Foundation
import SQLite3

final class ChannelRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

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
        let updateSQL = """
            UPDATE channels SET channel_id = ?, expected_usd = ?, stable_sats = ?,
                native_sats = ?, note = ?, receiver_sats = ?, latest_price = ?, updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ?
        """
        try rawSQL.execute(updateSQL, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(backingSats)),
            .integer(Int64(nativeSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)), .real(latestPrice),
            .text(userChannelId)
        ])

        if rawSQL.changes == 0 {
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
            try rawSQL.execute(insertSQL, params: [
                .text(channelId), .text(userChannelId), .real(expectedUSD),
                .integer(Int64(backingSats)), .integer(Int64(nativeSats)),
                note.map { .text($0) } ?? .null,
                .integer(Int64(receiverSats)), .real(latestPrice)
            ])
        }
    }

    /// Persist channel metadata without touching stable_sats.
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
        try rawSQL.execute(sql, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(nativeSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)),
            .real(latestPrice), .text(userChannelId)
        ])
        let changedRows = rawSQL.changes
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
            sql = "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, receiver_sats, latest_price, native_sats, sync_version FROM channels WHERE user_channel_id = ?"
            params = [.text(id)]
        } else {
            sql = """
                SELECT channel_id, expected_usd, note, stable_sats, user_channel_id,
                       receiver_sats, latest_price, native_sats, sync_version
                FROM channels
                ORDER BY updated_at DESC, channel_id DESC
                LIMIT 1
            """
            params = []
        }
        let rows = try rawSQL.query(sql, params: params)
        guard let row = rows.first else { return nil }

        return ChannelRecord(
            channelId: row.string(0),
            userChannelId: row.string(4),
            expectedUSD: row.double(1),
            note: row.optString(2),
            backingSats: row.uInt64(3),
            nativeSats: row.uInt64(7),
            receiverSats: row.uInt64(5),
            latestPrice: row.double(6),
            syncVersion: row.uInt64(8)
        )
    }

    func deleteChannel(userChannelId: String) throws {
        try rawSQL.execute("DELETE FROM channels WHERE user_channel_id = ?", params: [.text(userChannelId)])
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
        let sql = """
            INSERT INTO trades (channel_id, action, amount_usd, amount_btc, btc_price, fee_usd, payment_id, status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """
        try rawSQL.execute(sql, params: [
            .text(channelId), .text(action), .real(amountUSD), .real(amountBTC),
            .real(btcPrice), .real(feeUSD),
            paymentId.map { .text($0) } ?? .null, .text(status)
        ])
        return rawSQL.lastInsertRowId
    }

    func recordPreparedTrade(_ trade: PreparedMobileTrade) throws -> Int64 {
        guard trade.feeMsat <= UInt64(Int64.max),
              trade.newBackingSats <= UInt64(Int64.max),
              trade.createdAt <= UInt64(Int64.max),
              trade.expiresAt <= UInt64(Int64.max) else {
            throw DatabaseError.executeFailed("Prepared trade values exceed SQLite integer range")
        }
        return try rawSQL.inTransaction {
            let unresolved = try rawSQL.query(
                "SELECT 1 FROM trades WHERE channel_id = ? AND status IN ('prepared','sent','fee_paid','uncertain') LIMIT 1",
                params: [.text(trade.channelId)]
            )
            guard unresolved.isEmpty else {
                throw DatabaseError.executeFailed("A previous trade is still awaiting its signed result")
            }
            try rawSQL.execute(
                """
                INSERT INTO trades (
                    channel_id, user_channel_id, action, amount_usd, amount_btc, btc_price,
                    fee_usd, status, trade_id, request_hash, request_payload,
                    old_expected_usd, new_expected_usd, new_backing_sats, quote_price,
                    fee_msat, expires_at, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, 'prepared', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                params: [
                    .text(trade.channelId), .text(trade.userChannelId), .text(trade.action),
                    .real(trade.amountUSD), .real(trade.amountBTC), .real(trade.quotePrice),
                    .real(trade.feeUSD), .text(trade.tradeId), .text(trade.requestHash),
                    .text(trade.requestPayload), .real(trade.oldExpectedUSD),
                    .real(trade.newExpectedUSD), .integer(Int64(trade.newBackingSats)),
                    .real(trade.quotePrice), .integer(Int64(trade.feeMsat)),
                    .integer(Int64(trade.expiresAt)), .integer(Int64(trade.createdAt))
                ]
            )
            return rawSQL.lastInsertRowId
        }
    }

    func attachTradePaymentId(tradeDbId: Int64, paymentId: String) throws -> Bool {
        guard TradeProtocol.isCanonicalIdentifier(paymentId) else { return false }
        return try rawSQL.executeReturningChanges(
            """
            UPDATE trades SET payment_id = ?, trade_payment_id = ?, status = 'sent'
            WHERE id = ? AND status = 'prepared'
            """,
            params: [.text(paymentId), .text(paymentId), .integer(tradeDbId)]
        ) == 1
    }

    func markTradeFeePaid(paymentId: String) throws -> Bool {
        try rawSQL.executeReturningChanges(
            """
            UPDATE trades SET status = 'fee_paid'
            WHERE trade_payment_id = ? AND status IN ('prepared','sent','uncertain')
            """,
            params: [.text(paymentId)]
        ) == 1
    }

    func markKnownTradeFeePaid(tradeDbId: Int64, paymentId: String) throws -> Bool {
        guard TradeProtocol.isCanonicalIdentifier(paymentId) else { return false }
        return try rawSQL.executeReturningChanges(
            """
            UPDATE trades
            SET payment_id = ?, trade_payment_id = ?, status = 'fee_paid'
            WHERE id = ? AND status IN ('prepared','sent','uncertain')
              AND (trade_payment_id IS NULL OR trade_payment_id = ?)
            """,
            params: [
                .text(paymentId), .text(paymentId), .integer(tradeDbId), .text(paymentId)
            ]
        ) == 1
    }

    func tradePaymentExists(paymentId: String) throws -> Bool {
        try !rawSQL.query(
            "SELECT 1 FROM trades WHERE trade_payment_id = ? LIMIT 1",
            params: [.text(paymentId)]
        ).isEmpty
    }

    func hasUnattachedPreparedTrade() throws -> Bool {
        try !rawSQL.query(
            "SELECT 1 FROM trades WHERE trade_payment_id IS NULL AND status = 'prepared' LIMIT 1"
        ).isEmpty
    }

    func adoptUnattachedPreparedTrade(
        paymentId: String,
        amountMsat: UInt64,
        now: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) throws -> PendingTradePayment? {
        guard TradeProtocol.isCanonicalIdentifier(paymentId),
              amountMsat <= UInt64(Int64.max), now <= UInt64(Int64.max) else { return nil }
        return try rawSQL.inTransaction {
            let cutoff = Int64(max(now, TradeProtocol.responseRetryWindowSecs)
                - TradeProtocol.responseRetryWindowSecs)
            let rows = try rawSQL.query(
                """
                SELECT id, new_expected_usd, quote_price, action
                FROM trades
                WHERE trade_payment_id IS NULL AND status = 'prepared'
                  AND fee_msat = ? AND created_at >= ?
                ORDER BY id DESC LIMIT 2
                """,
                params: [.integer(Int64(amountMsat)), .integer(cutoff)]
            )
            guard rows.count == 1, let row = rows.first else { return nil }
            let tradeDbId = row.int64(0)
            let changed = try rawSQL.executeReturningChanges(
                """
                UPDATE trades SET payment_id = ?, trade_payment_id = ?, status = 'fee_paid'
                WHERE id = ? AND trade_payment_id IS NULL AND status = 'prepared'
                """,
                params: [.text(paymentId), .text(paymentId), .integer(tradeDbId)]
            )
            guard changed == 1 else {
                throw DatabaseError.executeFailed("Prepared trade payment adoption lost its race")
            }
            return PendingTradePayment(
                newExpectedUSD: row.double(1),
                price: row.double(2),
                tradeDbId: tradeDbId,
                action: row.string(3),
                status: "fee_paid"
            )
        }
    }

    func failUnattachedPreparedTrade(
        paymentId: String,
        amountMsat: UInt64,
        now: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) throws -> PendingTradePayment? {
        guard TradeProtocol.isCanonicalIdentifier(paymentId),
              amountMsat <= UInt64(Int64.max), now <= UInt64(Int64.max) else { return nil }
        return try rawSQL.inTransaction {
            let cutoff = Int64(max(now, TradeProtocol.responseRetryWindowSecs)
                - TradeProtocol.responseRetryWindowSecs)
            let rows = try rawSQL.query(
                """
                SELECT id, new_expected_usd, quote_price, action
                FROM trades
                WHERE trade_payment_id IS NULL AND status = 'prepared'
                  AND fee_msat = ? AND created_at >= ?
                ORDER BY id DESC LIMIT 2
                """,
                params: [.integer(Int64(amountMsat)), .integer(cutoff)]
            )
            guard rows.count == 1, let row = rows.first else { return nil }
            let tradeDbId = row.int64(0)
            let changed = try rawSQL.executeReturningChanges(
                """
                UPDATE trades
                SET payment_id = ?, trade_payment_id = ?, status = 'send_failed',
                    outcome = 'send_failed', resolved_at = strftime('%s', 'now')
                WHERE id = ? AND trade_payment_id IS NULL AND status = 'prepared'
                """,
                params: [.text(paymentId), .text(paymentId), .integer(tradeDbId)]
            )
            guard changed == 1 else {
                throw DatabaseError.executeFailed("Prepared trade failure adoption lost its race")
            }
            return PendingTradePayment(
                newExpectedUSD: row.double(1),
                price: row.double(2),
                tradeDbId: tradeDbId,
                action: row.string(3),
                status: "send_failed"
            )
        }
    }

    func markTradeSendFailed(tradeDbId: Int64) throws -> Bool {
        try rawSQL.executeReturningChanges(
            """
            UPDATE trades SET status = 'send_failed', outcome = 'send_failed',
                resolved_at = strftime('%s', 'now')
            WHERE id = ? AND status IN ('prepared','sent','uncertain')
            """,
            params: [.integer(tradeDbId)]
        ) == 1
    }

    func markExpiredTradesUncertain(now: UInt64 = UInt64(Date().timeIntervalSince1970)) throws -> Int {
        guard now <= UInt64(Int64.max) else { return 0 }
        return try rawSQL.executeReturningChanges(
            """
            UPDATE trades SET status = 'uncertain', uncertainty_reason = 'no_response'
            WHERE expires_at IS NOT NULL AND expires_at <= ?
              AND status IN ('prepared','sent','fee_paid')
            """,
            params: [.integer(Int64(now))]
        )
    }

    func markTradeResponseNotCommittable(_ message: TradeControlMessage) throws -> Bool {
        let channelId: String
        let correlation: TradeCorrelation
        switch message {
        case .sync(let sync):
            guard let syncCorrelation = sync.correlation else { return false }
            channelId = sync.channelId
            correlation = syncCorrelation
        case .rejected(let rejection):
            channelId = rejection.channelId
            correlation = rejection.correlation
        }
        return try rawSQL.executeReturningChanges(
            """
            UPDATE trades SET status = 'uncertain', uncertainty_reason = 'response_not_committable'
            WHERE channel_id = ? AND trade_id = ? AND request_hash = ?
              AND (trade_payment_id = ? OR trade_payment_id IS NULL)
              AND status IN ('prepared','sent','fee_paid','uncertain')
            """,
            params: [
                .text(channelId), .text(correlation.tradeId), .text(correlation.requestHash),
                .text(correlation.tradePaymentId)
            ]
        ) == 1
    }

    func unresolvedTradePayments() throws -> [String: PendingTradePayment] {
        let rows = try rawSQL.query(
            """
            SELECT trade_payment_id, new_expected_usd, quote_price, id, action, status
            FROM trades
            WHERE trade_payment_id IS NOT NULL
              AND status IN ('sent','fee_paid','uncertain')
            ORDER BY id
            """
        )
        return Dictionary(uniqueKeysWithValues: rows.map { row in
            (row.string(0), PendingTradePayment(
                newExpectedUSD: row.double(1),
                price: row.double(2),
                tradeDbId: row.int64(3),
                action: row.string(4),
                status: row.string(5)
            ))
        })
    }

    /// Terminal outcome for a trade's fee payment id, straight from SQLite — the source
    /// of truth that both the foreground handler and the NSE write. The in-memory outcome
    /// map alone misses results committed while the app was backgrounded or before a
    /// restart. Returns nil while the trade is unresolved.
    func terminalTradeOutcome(paymentId: String) throws -> (accepted: Bool, reasonCode: String?)? {
        let rows = try rawSQL.query(
            """
            SELECT status, reason_code FROM trades
            WHERE trade_payment_id = ? AND status IN ('accepted','rejected')
            ORDER BY id DESC LIMIT 1
            """,
            params: [.text(paymentId)]
        )
        guard let row = rows.first else { return nil }
        return (row.string(0) == "accepted", row.optString(1))
    }

    func applyCorrelatedTradeAcceptance(
        _ sync: TradeControlMessage.Sync
    ) -> TradeControlApplyResult {
        guard let correlation = sync.correlation else {
            return TradeControlApplyResult(status: .invalid)
        }
        do {
            return try rawSQL.inTransaction {
                let rows = try rawSQL.query(
                    """
                    SELECT id, channel_id, user_channel_id, trade_payment_id,
                           new_expected_usd, new_backing_sats, status, action
                    FROM trades
                    WHERE trade_id = ? AND request_hash = ?
                      AND (trade_payment_id = ? OR trade_payment_id IS NULL)
                    LIMIT 1
                    """,
                    params: [
                        .text(correlation.tradeId), .text(correlation.requestHash),
                        .text(correlation.tradePaymentId)
                    ]
                )
                guard let trade = rows.first else {
                    return TradeControlApplyResult(status: .invalid)
                }
                let storedPayment = trade.optString(3)
                let storedExpected = trade.double(4)
                guard trade.string(1) == sync.channelId,
                      trade.string(2) == sync.userChannelId,
                      storedPayment == nil || storedPayment == correlation.tradePaymentId,
                      abs(storedExpected - sync.expectedUSD) <= 0.000000001,
                      let storedBackingSigned = trade.optInt64(5), storedBackingSigned >= 0 else {
                    return TradeControlApplyResult(status: .invalid)
                }
                let storedBacking = UInt64(storedBackingSigned)
                let status = trade.string(6)
                let action = trade.string(7)
                if status == "accepted" {
                    return TradeControlApplyResult(
                        status: .duplicate,
                        localBackingSats: storedBacking,
                        peerBackingSats: sync.backingSats,
                        paymentId: correlation.tradePaymentId,
                        action: action
                    )
                }
                guard status != "rejected", status != "send_failed" else {
                    return TradeControlApplyResult(status: .invalid)
                }
                let channels = try rawSQL.query(
                    "SELECT channel_id, receiver_sats, sync_version, stable_sats FROM channels WHERE user_channel_id = ?",
                    params: [.text(sync.userChannelId)]
                )
                guard let channel = channels.first else {
                    return TradeControlApplyResult(status: .retry)
                }
                guard channel.string(0) == sync.channelId else {
                    return TradeControlApplyResult(status: .invalid)
                }
                let receiverSigned = channel.int64(1)
                let currentVersion = channel.int64(2)
                let currentBackingSigned = channel.int64(3)
                guard receiverSigned >= 0, currentBackingSigned >= 0,
                      sync.syncVersion <= UInt64(Int64.max) else {
                    return TradeControlApplyResult(status: .invalid)
                }
                let allocationApplied = Int64(sync.syncVersion) > currentVersion
                if allocationApplied {
                    guard storedBacking <= UInt64(receiverSigned) else {
                        return TradeControlApplyResult(status: .retry)
                    }
                    let native = UInt64(receiverSigned) - storedBacking
                    let channelChanges = try rawSQL.executeReturningChanges(
                        """
                        UPDATE channels
                        SET expected_usd = ?, stable_sats = ?, native_sats = ?, sync_version = ?,
                            updated_at = strftime('%s', 'now')
                        WHERE user_channel_id = ? AND channel_id = ? AND sync_version < ?
                        """,
                        params: [
                            .real(sync.expectedUSD), .integer(Int64(storedBacking)),
                            .integer(Int64(native)), .integer(Int64(sync.syncVersion)),
                            .text(sync.userChannelId), .text(sync.channelId),
                            .integer(Int64(sync.syncVersion))
                        ]
                    )
                    guard channelChanges == 1 else {
                        throw DatabaseError.executeFailed("Correlated sync channel update did not affect one row")
                    }
                }
                let tradeChanges = try rawSQL.executeReturningChanges(
                    """
                    UPDATE trades
                    SET payment_id = ?, trade_payment_id = ?, status = 'accepted',
                        outcome = 'accepted', resolved_at = strftime('%s', 'now'),
                        uncertainty_reason = NULL
                    WHERE id = ?
                    """,
                    params: [
                        .text(correlation.tradePaymentId), .text(correlation.tradePaymentId),
                        .integer(trade.int64(0))
                    ]
                )
                guard tradeChanges == 1 else {
                    throw DatabaseError.executeFailed("Correlated sync trade update did not affect one row")
                }
                return TradeControlApplyResult(
                    status: .applied,
                    localBackingSats: allocationApplied ? storedBacking : UInt64(currentBackingSigned),
                    peerBackingSats: sync.backingSats,
                    paymentId: correlation.tradePaymentId,
                    action: action,
                    allocationApplied: allocationApplied
                )
            }
        } catch {
            return TradeControlApplyResult(status: .retry)
        }
    }

    func applyTradeRejection(
        _ rejection: TradeControlMessage.Rejected
    ) -> TradeControlApplyResult {
        do {
            return try rawSQL.inTransaction {
                let rows = try rawSQL.query(
                    """
                    SELECT id, channel_id, trade_payment_id, status, action
                    FROM trades
                    WHERE trade_id = ? AND request_hash = ?
                      AND (trade_payment_id = ? OR trade_payment_id IS NULL)
                    LIMIT 1
                    """,
                    params: [
                        .text(rejection.correlation.tradeId),
                        .text(rejection.correlation.requestHash),
                        .text(rejection.correlation.tradePaymentId)
                    ]
                )
                guard let trade = rows.first,
                      trade.string(1) == rejection.channelId,
                      trade.optString(2) == nil || trade.optString(2) == rejection.correlation.tradePaymentId else {
                    return TradeControlApplyResult(status: .invalid)
                }
                if trade.string(3) == "rejected" {
                    return TradeControlApplyResult(
                        status: .duplicate,
                        paymentId: rejection.correlation.tradePaymentId,
                        action: trade.string(4)
                    )
                }
                guard trade.string(3) != "accepted", trade.string(3) != "send_failed",
                      rejection.decidedAt <= UInt64(Int64.max) else {
                    return TradeControlApplyResult(status: .invalid)
                }
                let changes = try rawSQL.executeReturningChanges(
                    """
                    UPDATE trades
                    SET payment_id = ?, trade_payment_id = ?, status = 'rejected',
                        outcome = 'rejected', reason_code = ?, resolved_at = ?,
                        uncertainty_reason = NULL
                    WHERE id = ?
                    """,
                    params: [
                        .text(rejection.correlation.tradePaymentId),
                        .text(rejection.correlation.tradePaymentId),
                        .text(rejection.reasonCode), .integer(Int64(rejection.decidedAt)),
                        .integer(trade.int64(0))
                    ]
                )
                guard changes == 1 else {
                    throw DatabaseError.executeFailed("Trade rejection did not affect one row")
                }
                return TradeControlApplyResult(
                    status: .applied,
                    paymentId: rejection.correlation.tradePaymentId,
                    action: trade.string(4)
                )
            }
        } catch {
            return TradeControlApplyResult(status: .retry)
        }
    }

    func applyUncorrelatedSyncIfNewer(
        _ sync: TradeControlMessage.Sync,
        trustedPrice: Double
    ) -> TradeControlApplyResult {
        guard sync.correlation == nil, trustedPrice.isFinite, trustedPrice > 0,
              sync.syncVersion <= UInt64(Int64.max) else {
            return TradeControlApplyResult(status: .invalid)
        }
        do {
            return try rawSQL.inTransaction {
                let rows = try rawSQL.query(
                    """
                    SELECT channel_id, expected_usd, stable_sats, receiver_sats, sync_version
                    FROM channels WHERE user_channel_id = ?
                    """,
                    params: [.text(sync.userChannelId)]
                )
                guard let channel = rows.first else {
                    return TradeControlApplyResult(status: .retry)
                }
                guard channel.string(0) == sync.channelId else {
                    return TradeControlApplyResult(status: .invalid)
                }
                if Int64(sync.syncVersion) <= channel.int64(4) {
                    return TradeControlApplyResult(status: .duplicate)
                }
                let currentBackingSigned = channel.int64(2)
                let receiverSigned = channel.int64(3)
                guard currentBackingSigned >= 0, receiverSigned >= 0 else {
                    return TradeControlApplyResult(status: .retry)
                }
                let currentBacking = UInt64(currentBackingSigned)
                let receiver = UInt64(receiverSigned)
                let localBacking: UInt64
                if sync.expectedUSD == 0 {
                    localBacking = 0
                } else if currentBacking > 0, sync.expectedUSD == channel.double(1) {
                    localBacking = min(currentBacking, receiver)
                } else {
                    guard let calculated = TradeProtocol.tradeBackingAfterDelta(
                        receiverSats: receiver,
                        currentBackingSats: currentBacking,
                        currentExpectedUSD: channel.double(1),
                        newExpectedUSD: sync.expectedUSD,
                        price: trustedPrice
                    ) else { return TradeControlApplyResult(status: .retry) }
                    localBacking = calculated
                }
                let native = receiver - localBacking
                let changes = try rawSQL.executeReturningChanges(
                    """
                    UPDATE channels
                    SET expected_usd = ?, stable_sats = ?, native_sats = ?, sync_version = ?,
                        latest_price = ?, updated_at = strftime('%s', 'now')
                    WHERE user_channel_id = ? AND channel_id = ? AND sync_version < ?
                    """,
                    params: [
                        .real(sync.expectedUSD), .integer(Int64(localBacking)),
                        .integer(Int64(native)), .integer(Int64(sync.syncVersion)),
                        .real(trustedPrice), .text(sync.userChannelId), .text(sync.channelId),
                        .integer(Int64(sync.syncVersion))
                    ]
                )
                guard changes == 1 else {
                    throw DatabaseError.executeFailed("Uncorrelated sync did not affect one row")
                }
                return TradeControlApplyResult(
                    status: .applied,
                    localBackingSats: localBacking,
                    peerBackingSats: sync.backingSats
                )
            }
        } catch {
            return TradeControlApplyResult(status: .retry)
        }
    }

    func getRecentTrades(limit: Int) throws -> [TradeRecord] {
        let sql = """
            SELECT id, channel_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                   payment_id, status, created_at
            FROM trades ORDER BY id DESC LIMIT ?
        """
        let rows = try rawSQL.query(sql, params: [.integer(Int64(limit))])
        return rows.map { row in
            TradeRecord(
                id: row.int64(0),
                channelId: row.string(1),
                action: row.string(2),
                amountUSD: row.double(3),
                amountBTC: row.double(4),
                btcPrice: row.double(5),
                feeUSD: row.double(6),
                paymentId: row.optString(7),
                status: row.string(8),
                createdAt: row.int64(9)
            )
        }
    }
}
