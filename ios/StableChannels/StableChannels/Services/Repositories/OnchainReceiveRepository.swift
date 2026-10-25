import Foundation
import SQLite3

final class OnchainReceiveRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
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

    // MARK: - Receive recording (websocket, recovery, wallet-list adoption)

    /// Receive rows joined to their resolution row, so a balance-delta placeholder (whose
    /// address lives on `onchain_receive_txids`) and a txid row (address on the payment)
    /// expose one address.
    private static let receiveRowsFrom = """
    FROM payments p
    LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
    WHERE p.payment_type = 'onchain' AND p.direction = 'received'
    """
    private static let rowAddress = "COALESCE(NULLIF(p.address, ''), r.address)"
    private static let isTxidless = "(p.txid IS NULL OR p.txid = '')"
    /// Bounds the placeholder scans; a wallet never has this many deposits pending at once.
    private static let maxPendingPlaceholders = 64

    static func receivePaymentId(txid: String) -> String {
        "onchain_receive_\(txid)"
    }

    /// Whether `txid` is already accounted for by a payment row, a resolution row, or the
    /// receive row id it would get.
    private func isTracked(txid: String) throws -> Bool {
        let rows = try rawSQL.query(
            """
            SELECT 1 FROM payments WHERE txid = ? OR payment_id = ?
            UNION ALL SELECT 1 FROM onchain_receive_txids WHERE txid = ?
            LIMIT 1
            """,
            params: [.text(txid), .text(Self.receivePaymentId(txid: txid)), .text(txid)]
        )
        return !rows.isEmpty
    }

    /// Records a websocket-detected or recovered receive unless its txid is already tracked.
    /// Check and write run in one transaction so a concurrent balance-delta detection can't
    /// double-insert. A txid-less placeholder for this address with exactly this amount is
    /// adopted instead of inserting a second row: for a single deposit the balance delta
    /// equals the vout sum, so a placeholder with another amount is a different deposit, or
    /// several merged into one (dropped here once their own rows add up to it). Returns true
    /// when a row was adopted or inserted.
    func recordReceive(
        txid: String,
        amountSats: Int64,
        amountUSD: Double?,
        btcPrice: Double?,
        address: String
    ) throws -> Bool {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            if try isTracked(txid: txid) { return false }
            let amountMsat = amountSats * 1000
            let placeholder = try rawSQL.query(
                """
                SELECT p.id \(Self.receiveRowsFrom)
                  AND p.status = 'pending' AND \(Self.isTxidless)
                  AND p.amount_msat = ? AND \(Self.rowAddress) = ?
                ORDER BY p.created_at DESC, p.id DESC LIMIT 1
                """,
                params: [.integer(amountMsat), .text(address)]
            ).first
            if let placeholder {
                let rowId = placeholder.int64(0)
                try attach(txid: txid, toRowId: rowId, address: address)
                try rawSQL.execute(
                    "UPDATE payments SET amount_usd = ?, btc_price = ? WHERE id = ?",
                    params: [
                        amountUSD.map { .real($0) } ?? .null,
                        btcPrice.map { .real($0) } ?? .null,
                        .integer(rowId)
                    ]
                )
                return true
            }
            try rawSQL.execute(
                """
                INSERT INTO payments (
                    payment_id, payment_type, direction, amount_msat,
                    amount_usd, btc_price, status, txid, address
                )
                VALUES (?, 'onchain', 'received', ?, ?, ?, 'pending', ?, ?)
                """,
                params: [
                    .text(Self.receivePaymentId(txid: txid)),
                    .integer(amountMsat),
                    amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    .text(txid),
                    .text(address)
                ]
            )
            try dropMergedPlaceholders(address: address)
            return true
        }
    }

    /// Records a deposit found by an address lookup. Skipped while a splice has no txid yet
    /// (a payment to our address could be that splice's self-send; retried on a later check),
    /// and when a row with this amount since `sinceSecs` already covers it: one carrying this
    /// txid, or an older txid-less one that is not a pending placeholder (the txid check alone
    /// would duplicate those). Pending placeholders are adopted by `recordReceive`.
    func recordMissedReceive(
        txid: String,
        amountSats: Int64,
        amountUSD: Double?,
        btcPrice: Double?,
        address: String,
        sinceSecs: Int64
    ) throws -> Bool {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            let spliceWithoutTxid = try rawSQL.query(SpliceEventRecorder.hasPendingSpliceWithoutTxidSQL)
            if !spliceWithoutTxid.isEmpty { return false }
            let duplicate = try rawSQL.query(
                """
                SELECT 1 FROM payments
                WHERE payment_type = 'onchain' AND direction = 'received'
                  AND amount_msat = ? AND created_at >= ?
                  AND status != 'failed'
                  AND (txid IS NULL OR txid = '' OR txid = ?)
                  AND NOT (status = 'pending' AND (txid IS NULL OR txid = ''))
                LIMIT 1
                """,
                params: [.integer(amountSats * 1000), .integer(sinceSecs), .text(txid)]
            )
            if !duplicate.isEmpty { return false }
            return try recordReceive(
                txid: txid,
                amountSats: amountSats,
                amountUSD: amountUSD,
                btcPrice: btcPrice,
                address: address
            )
        }
    }

    /// Attaches `txid` to a txid-less pending row and resolves its resolution row, so an
    /// in-flight address resolver can no longer attach another transaction to it.
    private func attach(txid: String, toRowId rowId: Int64, address: String) throws {
        try rawSQL.execute(
            """
            UPDATE onchain_receive_txids
            SET txid = ?, status = 'resolved', resolved_at = strftime('%s', 'now')
            WHERE status = 'pending' AND id = (SELECT resolution_id FROM payments WHERE id = ?)
            """,
            params: [.text(txid), .integer(rowId)]
        )
        try rawSQL.execute(
            """
            UPDATE payments SET payment_id = ?, txid = ?, address = ?
            WHERE id = ? AND status = 'pending' AND (txid IS NULL OR txid = '')
            """,
            params: [.text(Self.receivePaymentId(txid: txid)), .text(txid), .text(address), .integer(rowId)]
        )
    }

    /// Deletes txid-less pending placeholders on `address` that two or more txid rows recorded
    /// soon after them add up to (see `MergedReceivePolicy`), with their pending resolution rows.
    private func dropMergedPlaceholders(address: String) throws {
        let placeholders = try rawSQL.query(
            """
            SELECT p.id, p.amount_msat, p.created_at \(Self.receiveRowsFrom)
              AND p.status = 'pending' AND \(Self.isTxidless) AND \(Self.rowAddress) = ?
            ORDER BY p.created_at ASC LIMIT \(Self.maxPendingPlaceholders)
            """,
            params: [.text(address)]
        ).map { MergedReceivePolicy.Row(id: $0.int64(0), amountMsat: $0.int64(1), createdAt: $0.int64(2)) }
        guard let earliest = placeholders.first?.createdAt,
              let latest = placeholders.map(\.createdAt).max() else { return }
        let receives = try rawSQL.query(
            """
            SELECT p.id, p.amount_msat, p.created_at \(Self.receiveRowsFrom)
              AND NOT \(Self.isTxidless) AND p.status != 'failed' AND \(Self.rowAddress) = ?
              AND p.created_at >= ? AND p.created_at <= ?
            ORDER BY p.created_at ASC, p.id ASC LIMIT ?
            """,
            params: [
                .text(address),
                .integer(earliest),
                .integer(latest + MergedReceivePolicy.windowSeconds),
                .integer(Int64(placeholders.count * MergedReceivePolicy.maxRowsPerPlaceholder))
            ]
        ).map { MergedReceivePolicy.Row(id: $0.int64(0), amountMsat: $0.int64(1), createdAt: $0.int64(2)) }
        for id in MergedReceivePolicy.mergedPlaceholderIds(placeholders: placeholders, receives: receives) {
            try rawSQL.execute(
                """
                DELETE FROM onchain_receive_txids WHERE status = 'pending'
                  AND id = (SELECT resolution_id FROM payments WHERE id = ?)
                """,
                params: [.integer(id)]
            )
            try rawSQL.execute("DELETE FROM payments WHERE id = ?", params: [.integer(id)])
        }
    }

    /// Txid-less pending receives that have an address, oldest first, for wallet-list txid
    /// adoption. Rows without an address are never candidates.
    func txidlessPendingReceivesWithAddress() -> [ReceiveTxidAdoption.Placeholder] {
        let rows = (try? rawSQL.query(
            """
            SELECT p.id, \(Self.rowAddress), p.amount_msat, p.created_at \(Self.receiveRowsFrom)
              AND p.status = 'pending' AND \(Self.isTxidless)
              AND \(Self.rowAddress) IS NOT NULL AND \(Self.rowAddress) != ''
            ORDER BY p.created_at ASC LIMIT \(Self.maxPendingPlaceholders)
            """,
            params: []
        )) ?? []
        return rows.map {
            ReceiveTxidAdoption.Placeholder(
                id: $0.int64(0),
                address: $0.string(1),
                amountMsat: $0.int64(2),
                createdAt: $0.int64(3)
            )
        }
    }

    /// Attaches `txid` to the txid-less pending receive `rowId`. Does nothing if the row got a
    /// txid meanwhile or `txid` is already tracked. Returns true when the row was updated.
    func adoptTxid(rowId: Int64, txid: String, address: String) -> Bool {
        (try? rawSQL.inTransaction(mode: "IMMEDIATE") {
            if try isTracked(txid: txid) { return false }
            try attach(txid: txid, toRowId: rowId, address: address)
            return rawSQL.changes > 0
        }) ?? false
    }
}
