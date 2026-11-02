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

    /// The payment id of the receive row for [txid].
    static func receivePaymentId(txid: String) -> String {
        "onchain_receive_\(txid)"
    }

    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    /// Records a receive seen on the WebSocket, at most once per txid.
    ///
    /// Once the wallet balance has counted the transaction ([balanceCounted]), the balance fallback
    /// may have recorded it as a txid-less placeholder listing this txid as a candidate: one of
    /// exactly this amount takes the txid, and a larger one (several deposits in one balance jump)
    /// gives this amount up to a new row. Any other placeholder belongs to another deposit, so a new
    /// row is written, as it is before the balance counts the transaction.
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

            if balanceCounted,
               let placeholder = try fallbackRow(for: txid, covering: amountMsat, address: address) {
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

    /// The oldest pending, txid-less fallback row for [address] that lists [txid] as a candidate and
    /// is at least [amountMsat], preferring one of exactly that amount: (id, resolution_id, amount_msat).
    private func fallbackRow(for txid: String, covering amountMsat: UInt64, address: String) throws -> [Any?]? {
        try rawSQL.query(
            """
            SELECT p.id, p.resolution_id, p.amount_msat FROM payments p
            LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
            WHERE p.payment_type = 'onchain' AND p.direction = 'received' AND p.status = 'pending'
              AND p.txid IS NULL AND p.amount_msat >= ? AND (r.address IS NULL OR r.address = ?)
              AND EXISTS (
                  SELECT 1 FROM onchain_deposit_candidates c WHERE c.payment_id = p.payment_id AND c.txid = ?
              )
            ORDER BY p.amount_msat = ? DESC, p.id ASC LIMIT 1
            """,
            params: [.integer(Int64(amountMsat)), .text(address), .text(txid), .integer(Int64(amountMsat))]
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
                .text(Self.receivePaymentId(txid: txid)),
                .integer(Int64(amountMsat)),
                amountUSD.map { .real($0) } ?? .null,
                btcPrice.map { .real($0) } ?? .null,
                .text(txid),
                .text(address)
            ]
        ) > 0
    }

    /// Sets the receive row for [txid] to the node's amount, scaling its USD value with it. The row's
    /// amount came from the WebSocket feed.
    func correctReceiveAmount(txid: String, sats: Int64) throws {
        let msat = sats.multipliedReportingOverflow(by: 1000)
        guard sats > 0, !msat.overflow else { return }
        try rawSQL.execute(
            """
            UPDATE payments
            SET amount_usd = amount_usd * ? / amount_msat, amount_msat = ?
            WHERE payment_type = 'onchain' AND direction = 'received' AND txid = ? AND amount_msat > 0
            """,
            params: [.integer(msat.partialValue), .integer(msat.partialValue), .text(txid)]
        )
    }

    /// Sats of the on-chain receives already recorded for [txids], by txid, failed ones excluded,
    /// and only those recorded at or after [recordedSince] when given.
    /// The partial unique index allows one receive row per txid.
    func recordedReceiveSats(txids: Set<String>, recordedSince: Date? = nil) throws -> [String: Int64] {
        guard !txids.isEmpty else { return [:] }
        let marks = Array(repeating: "?", count: txids.count).joined(separator: ", ")
        let since = Int64(recordedSince?.timeIntervalSince1970.rounded(.down) ?? 0)
        return try rawSQL.query(
            """
            SELECT txid, amount_msat FROM payments
            WHERE payment_type = 'onchain' AND direction = 'received' AND status != 'failed'
              AND created_at >= ? AND txid IN (\(marks))
            """,
            params: [.integer(since)] + txids.sorted().map { .text($0) }
        ).reduce(into: [:]) { sats, row in
            if let txid = row.optString(0) { sats[txid] = row.int64(1) / 1000 }
        }
    }

    /// Records the txids that could be the balance-fallback deposit [paymentId]: the arrivals of its
    /// balance jump no recorded receive explained. Only these may later take the row over.
    func recordDepositCandidates(paymentId: String, txids: Set<String>) throws {
        for txid in txids.sorted() {
            try rawSQL.execute(
                "INSERT OR IGNORE INTO onchain_deposit_candidates (payment_id, txid) VALUES (?, ?)",
                params: [.text(paymentId), .text(txid)]
            )
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
