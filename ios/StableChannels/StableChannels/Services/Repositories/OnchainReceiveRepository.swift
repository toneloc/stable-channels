import Foundation
import SQLite3

final class OnchainReceiveRepository {
    enum WebSocketReceiveOutcome: Equatable {
        case alreadyRecorded
        /// The balance fallback had already recorded this deposit; its row now carries the txid.
        case adoptedPlaceholder
        case inserted
    }

    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    /// Records a receive seen on the WebSocket, at most once per txid. A pending placeholder of
    /// exactly this amount means the balance fallback already counted the deposit, so it gets
    /// the txid instead of a second row being written. Otherwise a new row is inserted and marked
    /// as awaiting the balance, so the fallback doesn't count it again when the node syncs.
    func recordWebSocketReceive(
        txid: String,
        address: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?
    ) throws -> WebSocketReceiveOutcome {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            let existing = try rawSQL.query(
                "SELECT 1 FROM payments WHERE txid = ? LIMIT 1",
                params: [.text(txid)]
            )
            if !existing.isEmpty { return .alreadyRecorded }

            let placeholders = try rawSQL.query(
                """
                SELECT p.id, p.resolution_id FROM payments p
                LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
                WHERE p.payment_type = 'onchain' AND p.direction = 'received' AND p.status = 'pending'
                  AND p.txid IS NULL AND p.amount_msat = ? AND (r.address IS NULL OR r.address = ?)
                ORDER BY p.id ASC LIMIT 1
                """,
                params: [.integer(Int64(amountMsat)), .text(address)]
            )
            if let placeholder = placeholders.first {
                if let resolutionId = placeholder.optInt64(1) {
                    // Stops the txid resolver: the deposit is identified.
                    try rawSQL.execute(
                        """
                        UPDATE onchain_receive_txids
                        SET txid = ?, status = 'resolved', resolved_at = strftime('%s', 'now')
                        WHERE id = ? AND status = 'pending'
                        """,
                        params: [.text(txid), .integer(resolutionId)]
                    )
                }
                try rawSQL.execute(
                    "UPDATE payments SET txid = ?, address = ? WHERE id = ?",
                    params: [.text(txid), .text(address), .integer(placeholder.int64(0))]
                )
                return .adoptedPlaceholder
            }

            let inserted = try rawSQL.executeReturningChanges(
                """
                INSERT OR IGNORE INTO payments (
                    payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
                    status, txid, address, awaiting_balance
                )
                VALUES (?, 'onchain', 'received', ?, ?, ?, 'pending', ?, ?, 1)
                """,
                params: [
                    .text("onchain_receive_\(txid)"),
                    .integer(Int64(amountMsat)),
                    amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    .text(txid),
                    .text(address)
                ]
            )
            return inserted > 0 ? .inserted : .alreadyRecorded
        }
    }

    /// WebSocket receives the wallet balance hasn't shown yet, oldest first. A receive whose
    /// transaction was replaced or dropped is failed and never reaches the balance, so it is left out.
    func awaitingBalanceReceives() throws -> [AwaitingReceive] {
        try rawSQL.query(
            """
            SELECT id, amount_msat FROM payments
            WHERE awaiting_balance = 1 AND status != 'failed'
            ORDER BY id ASC
            """,
            params: []
        ).map { AwaitingReceive(rowId: $0.int64(0), sats: $0.int64(1) / 1000) }
    }

    func markBalanceSeen(rowIds: [Int64]) throws {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            for rowId in rowIds {
                try rawSQL.execute(
                    "UPDATE payments SET awaiting_balance = 0 WHERE id = ?",
                    params: [.integer(rowId)]
                )
            }
        }
    }

    @discardableResult
    func insertOnchainReceiveResolution(address: String) -> Int64? {
        do {
            try rawSQL.execute(
                """
                INSERT INTO onchain_receive_txids (address, status)
                VALUES (?, 'pending')
                """,
                params: [.text(address)]
            )
            return rawSQL.lastInsertRowId
        } catch {
            AuditService.log("DB_INSERT_RECEIVE_RES_FAILED", data: ["error": "\(error)"])
            return nil
        }
    }

    func fetchPendingOnchainReceives() -> [OnchainReceiveResolution] {
        do {
            let rows = try rawSQL.query(
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
                    id: row.int64(0),
                    address: row.string(1),
                    txid: row.optString(2),
                    status: row.string(3, default: "pending"),
                    createdAt: row.int64(4),
                    resolvedAt: row.optInt64(5)
                )
            }
        } catch {
            return []
        }
    }

    @discardableResult
    func updateOnchainReceiveResolution(id: Int64, txid: String) -> Bool {
        do {
            try rawSQL.execute(
                """
                UPDATE onchain_receive_txids
                SET txid = ?, status = 'resolved', resolved_at = strftime('%s', 'now')
                WHERE id = ? AND status = 'pending'
                  AND NOT EXISTS (SELECT 1 FROM payments WHERE txid = ?)
                  AND NOT EXISTS (SELECT 1 FROM onchain_receive_txids WHERE txid = ?)
                """,
                params: [.text(txid), .integer(id), .text(txid), .text(txid)]
            )
            return rawSQL.changes > 0
        } catch {
            return false
        }
    }

    /// Throw on read failure: an unavailable history must never look like an empty history.
    func recordedReceiveTxids() throws -> Set<String> {
        let rows = try rawSQL.query(
            """
            SELECT txid FROM payments WHERE txid IS NOT NULL
            UNION SELECT txid FROM onchain_receive_txids WHERE txid IS NOT NULL
            """,
            params: []
        )
        return Set(rows.map { $0.string(0) })
    }

    func fetchPendingOnchainReceiveRows() -> [PendingOnchainPayment] {
        do {
            let rows = try rawSQL.query(
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
                    paymentId: row.string(0),
                    amountMsat: row.int64(1),
                    createdAt: row.int64(2)
                )
            }
        } catch {
            return []
        }
    }

    @discardableResult
    func updatePaymentResolution(paymentId: String, resolutionId: Int64) -> Bool {
        do {
            try rawSQL.execute(
                "UPDATE payments SET resolution_id = ? WHERE payment_id = ?",
                params: [.integer(resolutionId), .text(paymentId)]
            )
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    func recordOnchainPaymentWithResolution(
        paymentId: String,
        amountMsat: Int64,
        amountUSD: Double?,
        btcPrice: Double?,
        resolutionId: Int64
    ) -> Bool {
        do {
            try rawSQL.execute(
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

    func fetchPendingOnchainReceiveRow(resolutionId: Int64) -> PendingOnchainPayment? {
        do {
            let rows = try rawSQL.query(
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
                paymentId: row.string(0),
                amountMsat: row.int64(1),
                createdAt: row.int64(2)
            )
        } catch {
            return nil
        }
    }

    func fetchLatestResolvedOnchainTxid() -> String? {
        do {
            let rows = try rawSQL.query(
                """
                SELECT txid FROM onchain_receive_txids
                WHERE status = 'resolved' AND txid IS NOT NULL
                ORDER BY resolved_at DESC, id DESC LIMIT 1
                """,
                params: []
            )
            return rows.first?.optString(0)
        } catch {
            return nil
        }
    }

    @discardableResult
    func deleteOnchainReceiveResolution(id: Int64) -> Bool {
        do {
            try rawSQL.execute(
                "DELETE FROM onchain_receive_txids WHERE id = ?",
                params: [.integer(id)]
            )
            return true
        } catch {
            return false
        }
    }
}
