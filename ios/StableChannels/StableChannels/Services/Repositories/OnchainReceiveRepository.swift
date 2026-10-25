import Foundation
import SQLite3

final class OnchainReceiveRepository {
    enum WebSocketReceiveOutcome: Equatable {
        case alreadyRecorded
        /// The balance fallback had recorded exactly this deposit; its row now carries the txid.
        case adoptedPlaceholder
        /// The balance fallback had recorded this deposit inside a larger one; it is split out.
        case splitFromPlaceholder
        case inserted
    }

    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    /// Records a receive seen on the WebSocket, at most once per txid.
    ///
    /// Once the wallet balance has counted the transaction ([balanceCounted]), the balance fallback
    /// recorded it as a txid-less placeholder: one of exactly this amount takes the txid, and a
    /// larger one (several deposits in one balance jump) gives this amount up to a new row. Before
    /// the balance counts it, no placeholder can be this deposit, so a new row is written and the
    /// fallback later subtracts it by txid.
    func recordWebSocketReceive(
        txid: String,
        address: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        balanceCounted: Bool
    ) throws -> WebSocketReceiveOutcome {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            let existing = try rawSQL.query(
                "SELECT 1 FROM payments WHERE txid = ? LIMIT 1",
                params: [.text(txid)]
            )
            if !existing.isEmpty { return .alreadyRecorded }

            if balanceCounted, let placeholder = try fallbackRow(covering: amountMsat, address: address) {
                let rowId = placeholder.int64(0)
                if placeholder.int64(2) == Int64(amountMsat) {
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
                        params: [.text(txid), .text(address), .integer(rowId)]
                    )
                    return .adoptedPlaceholder
                }
                // ponytail: assumes the larger placeholder holds this deposit. A receive the close
                // branch absorbed instead could shrink an unrelated placeholder; tying placeholders to
                // the txids of their balance jump would remove that.
                try rawSQL.execute(
                    """
                    UPDATE payments
                    SET amount_usd = amount_usd * (amount_msat - ?) / amount_msat,
                        amount_msat = amount_msat - ?
                    WHERE id = ?
                    """,
                    params: [.integer(Int64(amountMsat)), .integer(Int64(amountMsat)), .integer(rowId)]
                )
                try insertReceive(
                    txid: txid,
                    address: address,
                    amountMsat: amountMsat,
                    amountUSD: amountUSD,
                    btcPrice: btcPrice
                )
                return .splitFromPlaceholder
            }

            let inserted = try insertReceive(
                txid: txid,
                address: address,
                amountMsat: amountMsat,
                amountUSD: amountUSD,
                btcPrice: btcPrice
            )
            return inserted ? .inserted : .alreadyRecorded
        }
    }

    /// The oldest pending, txid-less fallback row for [address] that is at least [amountMsat],
    /// preferring one of exactly that amount: (id, resolution_id, amount_msat).
    private func fallbackRow(covering amountMsat: UInt64, address: String) throws -> [Any?]? {
        try rawSQL.query(
            """
            SELECT p.id, p.resolution_id, p.amount_msat FROM payments p
            LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
            WHERE p.payment_type = 'onchain' AND p.direction = 'received' AND p.status = 'pending'
              AND p.txid IS NULL AND p.amount_msat >= ? AND (r.address IS NULL OR r.address = ?)
            ORDER BY p.amount_msat = ? DESC, p.id ASC LIMIT 1
            """,
            params: [.integer(Int64(amountMsat)), .text(address), .integer(Int64(amountMsat))]
        ).first
    }

    @discardableResult
    private func insertReceive(
        txid: String,
        address: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?
    ) throws -> Bool {
        try rawSQL.executeReturningChanges(
            """
            INSERT OR IGNORE INTO payments (
                payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status, txid, address
            )
            VALUES (?, 'onchain', 'received', ?, ?, ?, 'pending', ?, ?)
            """,
            params: [
                .text("onchain_receive_\(txid)"),
                .integer(Int64(amountMsat)),
                amountUSD.map { .real($0) } ?? .null,
                btcPrice.map { .real($0) } ?? .null,
                .text(txid),
                .text(address)
            ]
        ) > 0
    }

    /// Sats of the on-chain receives already recorded for [txids], failed ones excluded.
    func recordedReceiveSats(txids: Set<String>) throws -> [Int64] {
        guard !txids.isEmpty else { return [] }
        let marks = Array(repeating: "?", count: txids.count).joined(separator: ", ")
        return try rawSQL.query(
            """
            SELECT amount_msat FROM payments
            WHERE payment_type = 'onchain' AND direction = 'received' AND status != 'failed'
              AND txid IN (\(marks))
            """,
            params: txids.sorted().map { .text($0) }
        ).map { $0.int64(0) / 1000 }
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
