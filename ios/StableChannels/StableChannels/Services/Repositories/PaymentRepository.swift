import Foundation
import SQLite3

final class PaymentRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    func paymentExists(txid: String, excludePaymentId: String) -> Bool {
        do {
            let rows = try rawSQL.query(
                "SELECT 1 FROM payments WHERE txid = ? AND payment_id != ? LIMIT 1",
                params: [.text(txid), .text(excludePaymentId)]
            )
            return !rows.isEmpty
        } catch {
            return false
        }
    }

    func deletePayment(paymentId: String) {
        do {
            try rawSQL.execute("DELETE FROM payments WHERE payment_id = ?", params: [.text(paymentId)])
        } catch {
            // Ignore
        }
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
        // INSERT OR IGNORE is atomic: the partial unique index on payment_id
        // (WHERE payment_id IS NOT NULL) enforces dedup at the DB level.
        // rawSQL.changes == 0 means a duplicate was silently ignored.
        let sql = """
            INSERT OR IGNORE INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, txid, address)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """
        try rawSQL.execute(sql, params: [
            paymentId.map { .text($0) } ?? .null,
            .text(paymentType), .text(direction), .integer(Int64(amountMsat)),
            amountUSD.map { .real($0) } ?? .null,
            btcPrice.map { .real($0) } ?? .null,
            counterparty.map { .text($0) } ?? .null,
            .text(status),
            txid.map { .text($0) } ?? .null,
            address.map { .text($0) } ?? .null
        ])
        return rawSQL.changes > 0
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
        try rawSQL.execute("BEGIN IMMEDIATE")
        do {
            // INSERT OR IGNORE: the partial unique index on payment_id enforces dedup atomically.
            // changes == 0 means this payment_id was already recorded (duplicate LDK event).
            try rawSQL.execute(
                "INSERT OR IGNORE INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params: [
                    paymentId.map { .text($0) } ?? .null,
                    .text(paymentType), .text(direction), .integer(Int64(amountMsat)),
                    amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    .text(status)
                ]
            )
            if rawSQL.changes == 0 {
                // Duplicate payment_id — already persisted
                let backing = try authoritativeBacking(
                    userChannelId: userChannelId,
                    required: backingDeltaSats != nil
                )
                try rawSQL.execute("ROLLBACK")
                return PaymentPersistenceResult(isNewPayment: false, backingSats: backing)
            }
            var resultingBacking: UInt64?
            if let delta = backingDeltaSats {
                guard let ucid = userChannelId, !ucid.isEmpty else {
                    throw DatabaseError.executeFailed("userChannelId required for backing update")
                }
                let rows = try rawSQL.query(
                    "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
                    params: [.text(ucid)]
                )
                guard let current = rows.first?.int64(0) else {
                    throw DatabaseError.missingChannelRow(ucid)
                }
                let newBacking = max(0, current + delta)
                if current + delta < 0 {
                    AuditService.log("BACKING_CLAMPED", data: [
                        "user_channel_id": ucid,
                        "current_backing_sats": "\(current)",
                        "delta_sats": "\(delta)"
                    ])
                }
                try rawSQL.execute(
                    "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s', 'now') WHERE user_channel_id = ?",
                    params: [.integer(newBacking), .text(ucid)]
                )
                let changedRows = rawSQL.changes
                if changedRows != 1 {
                    throw DatabaseError.executeFailed(
                        "backing UPDATE affected \(changedRows) rows for user_channel_id=\(ucid)"
                    )
                }
                resultingBacking = UInt64(newBacking)
            }
            try rawSQL.execute("COMMIT")
            return PaymentPersistenceResult(
                isNewPayment: true,
                backingSats: resultingBacking
            )
        } catch {
            try? rawSQL.execute("ROLLBACK")
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
        let rows = try rawSQL.query(
            "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
            params: [.text(ucid)]
        )
        guard let value = rows.first?.int64(0) else {
            throw DatabaseError.missingChannelRow(ucid)
        }
        guard value >= 0 else {
            throw DatabaseError.executeFailed(
                "No valid backing row for user_channel_id=\(ucid)"
            )
        }
        return UInt64(value)
    }

    func claimPendingSend(amountMsat: UInt64, price: Double) -> Bool {
        do {
            return try rawSQL.inTransaction(mode: "IMMEDIATE") {
                let existing = try rawSQL.query("SELECT id FROM pending_stability_send WHERE id = 1")
                guard existing.isEmpty else { return false }
                try rawSQL.execute(
                    "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)",
                    params: [
                        .integer(Int64(amountMsat)),
                        .real(price),
                        .integer(Int64(Date().timeIntervalSince1970))
                    ]
                )
                return true
            }
        } catch {
            return false
        }
    }

    @discardableResult
    func setPendingSendPaymentId(_ paymentId: String) -> Bool {
        do {
            try rawSQL.execute(
                "UPDATE pending_stability_send SET payment_id = ? WHERE id = 1",
                params: [.text(paymentId)]
            )
            return true
        } catch {
            return false
        }
    }

    func loadPendingSend() -> PendingStabilitySend? {
        guard let rows = try? rawSQL.query(
            "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1"
        ), let row = rows.first else {
            return nil
        }
        return PendingStabilitySend(
            paymentId: row.string(0),
            amountMsat: row.uInt64(1),
            price: row.double(2),
            createdAt: row.int64(3)
        )
    }

    func clearPendingSend() {
        try? rawSQL.execute("DELETE FROM pending_stability_send WHERE id = 1")
    }

    private func paymentRecord(from row: [Any?]) -> PaymentRecord {
        PaymentRecord(
            id: row.int64(0),
            paymentId: row.optString(1),
            paymentType: row.string(2, default: "manual"),
            direction: row.string(3),
            amountMsat: row.uInt64(4),
            amountUSD: row.optDouble(5),
            btcPrice: row.optDouble(6),
            counterparty: row.optString(7),
            status: row.string(8),
            createdAt: row.int64(9),
            feeMsat: row.uInt64(10),
            txid: row.optString(11),
            address: row.optString(12),
            confirmations: row.uInt32(13),
            txBlockHeight: row.optUInt32(14)
        )
    }

    func latestReceivedPayment() -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE direction = "received"
        AND NOT (payment_type = 'lightning' AND amount_msat < 1000)
        ORDER BY id DESC LIMIT 1
        """
        guard let row = try? rawSQL.query(sql, params: []).first else { return nil }
        return paymentRecord(from: row)
    }

    func payment(paymentId: String) -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE payment_id = ?
        ORDER BY id DESC LIMIT 1
        """
        guard let row = try? rawSQL.query(sql, params: [.text(paymentId)]).first else { return nil }
        return paymentRecord(from: row)
    }

    func paymentsNeedingConfirmation() throws -> [PaymentRecord] {
        let required = ConfirmationPolicy.requiredConfirmations
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE txid IS NOT NULL
        AND txid != ''
        AND payment_type IN ('onchain', 'splice_in', 'splice_out', 'channel_close')
        AND status != 'failed'
        AND (confirmations IS NULL OR confirmations < ?)
        ORDER BY created_at DESC
        LIMIT 50
        """
        let rows = try rawSQL.query(
            sql,
            params: [.integer(Int64(required))]
        )
        return rows.map { row in
            paymentRecord(from: row)
        }
    }

    func getPayment(byId id: Int64) throws -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments WHERE id = ? LIMIT 1
        """
        let rows = try rawSQL.query(sql, params: [.integer(id)])
        guard let row = rows.first else { return nil }
        return paymentRecord(from: row)
    }

    func updateConfirmations(paymentId: Int64, txBlockHeight: UInt32, currentBlockHeight: UInt32) throws {
        let required = ConfirmationPolicy.requiredConfirmations
        let rawConfs = max(Int(currentBlockHeight) - Int(txBlockHeight) + 1, 0)
        let confs = min(rawConfs, required)
        try rawSQL.execute(
            "UPDATE payments SET confirmations = ?, tx_block_height = ?, status = CASE WHEN ? >= ? THEN 'completed' ELSE status END WHERE id = ?",
            params: [
                .integer(Int64(confs)),
                .integer(Int64(txBlockHeight)),
                .integer(Int64(confs)),
                .integer(Int64(required)),
                .integer(paymentId)
            ]
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
        let rows = try rawSQL.query(sql, params: [.integer(Int64(limit))])
        return rows.map { row in
            paymentRecord(from: row)
        }
    }

    func updateTradeStatus(_ tradeId: Int64, status: String) throws {
        try rawSQL.execute(
            "UPDATE trades SET status = ? WHERE id = ?",
            params: [.text(status), .integer(tradeId)]
        )
    }

    func setPendingSpliceTxid(_ txid: String) throws {
        try rawSQL.execute(
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
        let rows = try rawSQL.query(
            "SELECT txid FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') AND txid IS NOT NULL ORDER BY id DESC LIMIT 1"
        )
        return rows.first?.optString(0)
    }

    func hasPendingSplice() throws -> Bool {
        let noTxidCutoff = Int64(Date().timeIntervalSince1970) - 600
        try rawSQL.execute(
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
        let rows = try rawSQL.query(
            "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') LIMIT 1"
        )
        return !rows.isEmpty
    }

    @discardableResult
    func completeLatestSplice(txid: String?) -> Bool {
        do {
            if let txid, !txid.isEmpty {
                try rawSQL.execute(
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
                try rawSQL.execute(
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

    @discardableResult
    func completeSplice(txid: String) -> Bool {
        do {
            try rawSQL.execute(
                """
                UPDATE payments
                SET status = 'completed', confirmations = 1
                WHERE payment_type IN ('splice_in', 'splice_out')
                  AND txid = ?
                  AND status IN ('pending', 'failed')
                """,
                params: [.text(txid)]
            )
            return rawSQL.changes > 0
        } catch {
            return false
        }
    }

    @discardableResult
    func failLatestPendingSplice() -> Bool {
        do {
            try rawSQL.execute(
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
            try rawSQL.execute(
                "UPDATE payments SET status = ?, fee_msat = ? WHERE payment_id = ? AND status = 'pending'",
                params: [.text(status), .integer(Int64(fee)), .text(paymentId)]
            )
        } else {
            try rawSQL.execute(
                "UPDATE payments SET status = ? WHERE payment_id = ? AND status = 'pending'",
                params: [.text(status), .text(paymentId)]
            )
        }
    }

    @discardableResult
    func updatePaymentTxid(paymentId: String, txid: String, status: String) -> Bool {
        do {
            try rawSQL.execute(
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

    func failPaymentByTxid(txid: String) throws {
        try rawSQL.execute(
            "UPDATE payments SET status = 'failed' WHERE txid = ? AND status = 'pending'",
            params: [.text(txid)]
        )
    }

    func isOutgoingStabilityPayment(paymentId: String) throws -> Bool {
        let rows = try rawSQL.query(
            "SELECT 1 FROM payments WHERE payment_id = ? AND payment_type = 'stability' AND direction = 'sent' LIMIT 1",
            params: [.text(paymentId)]
        )
        return !rows.isEmpty
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
        do {
            try rawSQL.execute(
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

    @discardableResult
    func updatePendingOperation(opId: String, closingTxid: String, status: String) -> Bool {
        do {
            try rawSQL.execute(
                """
                UPDATE pending_operations
                SET closing_txid = ?, status = ?, resolved_at = strftime('%s', 'now')
                WHERE op_id = ? AND status = 'pending'
                """,
                params: [.text(closingTxid), .text(status), .text(opId)]
            )
            return rawSQL.changes > 0
        } catch {
            return false
        }
    }

    func fetchPendingOperations() -> [PendingOperation] {
        do {
            let rows = try rawSQL.query(
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

    func fetchPendingOperation(opId: String) -> PendingOperation? {
        do {
            let rows = try rawSQL.query(
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

    func fetchPendingOperationByFundingTxid(_ txid: String) -> PendingOperation? {
        do {
            let rows = try rawSQL.query(
                """
                SELECT op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                       closing_txid, balance_sats, balance_usd, btc_price, counterparty,
                       status, created_at, resolved_at
                FROM pending_operations
                WHERE funding_outpoint_txid = ? AND status = 'pending'
                LIMIT 1
                """,
                params: [.text(txid)]
            )
            return rows.first.map { Self.parsePendingOperation($0) }
        } catch {
            return nil
        }
    }

    private static func parsePendingOperation(_ row: [Any?]) -> PendingOperation {
        PendingOperation(
            opId: row.string(0),
            opType: row.string(1),
            fundingOutpointTxid: row.optString(2),
            fundingOutpointVout: row.optUInt32(3),
            closingTxid: row.optString(4),
            balanceSats: row.optUInt64(5),
            balanceUsd: row.optDouble(6),
            btcPrice: row.optDouble(7),
            counterparty: row.optString(8),
            status: row.string(9, default: "pending"),
            createdAt: row.int64(10),
            resolvedAt: row.optInt64(11)
        )
    }
}
