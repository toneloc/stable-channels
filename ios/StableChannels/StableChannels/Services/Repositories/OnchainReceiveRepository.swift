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

    /// Records a receive seen on the WebSocket, at most once per txid. See ``recordReceive``.
    func recordWebSocketReceive(
        txid: String,
        address: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        balanceCounted: Bool
    ) throws -> WebSocketReceiveOutcome {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            try recordReceive(
                txid: txid,
                address: address,
                amountMsat: amountMsat,
                amountUSD: amountUSD,
                btcPrice: btcPrice,
                balanceCounted: balanceCounted
            )
        }
    }

    /// Records a deposit found by an address lookup, at most once per txid. Skipped while a splice
    /// has no txid yet ([hasSpliceWithoutTxid]: a payment to our address could be that splice's
    /// self-send; retried on a later check), for an amount no transaction can have, and when a row
    /// already covers it: one carrying this txid, or a placeholder since [sinceSecs] that was
    /// completed without a txid but lists this txid as a candidate. An equal amount alone is never
    /// evidence. Pending placeholders are handled by ``recordReceive``. Returns true when a row took
    /// the deposit.
    func recordMissedReceive(
        txid: String,
        amountSats: Int64,
        amountUSD: Double?,
        btcPrice: Double?,
        address: String,
        sinceSecs: Int64,
        balanceCounted: Bool,
        hasSpliceWithoutTxid: Bool
    ) throws -> Bool {
        guard !hasSpliceWithoutTxid, let amountMsat = OnchainAmount.msats(fromSats: amountSats) else { return false }
        return try rawSQL.inTransaction(mode: "IMMEDIATE") {
            let duplicate = try rawSQL.query(
                """
                SELECT 1 FROM payments
                WHERE \(Self.isReceiveRow())
                  AND created_at >= ? AND status NOT IN ('pending', 'failed') AND \(Self.lacksTxid)
                  AND \(Self.listsCandidate())
                LIMIT 1
                """,
                params: [.integer(sinceSecs), .text(txid)]
            )
            if !duplicate.isEmpty { return false }
            return try recordReceive(
                txid: txid,
                address: address,
                amountMsat: amountMsat,
                amountUSD: amountUSD,
                btcPrice: btcPrice,
                balanceCounted: balanceCounted
            ) != .alreadyRecorded
        }
    }

    /// The one deposit recorder, for the WebSocket and the address-lookup recovery: at most one row
    /// per txid. Must run inside the caller's transaction.
    ///
    /// Once the wallet balance has counted the transaction ([balanceCounted]), the balance fallback
    /// may have recorded it as a placeholder listing this txid as a candidate: one of exactly this
    /// amount takes the txid, and a larger one (several deposits in one balance jump) gives this
    /// amount up to a new row. Any other placeholder belongs to another deposit, so a new row is
    /// written, as it is before the balance counts the transaction. An equal amount alone is never
    /// evidence.
    private func recordReceive(
        txid: String,
        address: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        balanceCounted: Bool
    ) throws -> WebSocketReceiveOutcome {
        if try isRecorded(txid: txid) { return .alreadyRecorded }
        if balanceCounted, let placeholder = try candidatePlaceholder(
            for: txid,
            covering: amountMsat,
            address: address
        ) {
            if placeholder.amountMsat == Int64(amountMsat) {
                try attach(txid: txid, toRowId: placeholder.id, address: address)
                return .adoptedPlaceholder
            }
            try shrinkPlaceholder(rowId: placeholder.id, byMsat: Int64(amountMsat))
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

    /// Gives the placeholder [rowId] the txid of a deposit the wallet list or the address resolver
    /// identified, by the same rule as ``recordReceive``: the placeholder lists [txid] as a
    /// candidate, is on [address] and is of exactly [amountMsat], and no row carries [txid] yet.
    /// Returns true when the placeholder took the txid.
    func attachTxid(rowId: Int64, txid: String, address: String, amountMsat: Int64) throws -> Bool {
        guard amountMsat > 0 else { return false }
        return try rawSQL.inTransaction(mode: "IMMEDIATE") {
            guard try !isRecorded(txid: txid),
                  let placeholder = try candidatePlaceholder(
                      for: txid,
                      covering: UInt64(amountMsat),
                      address: address,
                      rowId: rowId
                  ),
                  placeholder.amountMsat == amountMsat else { return false }
            try attach(txid: txid, toRowId: rowId, address: address)
            return true
        }
    }

    /// Whether a payment row already carries [txid].
    private func isRecorded(txid: String) throws -> Bool {
        try !rawSQL.query("SELECT 1 FROM payments WHERE txid = ? LIMIT 1", params: [.text(txid)]).isEmpty
    }

    /// A row without a txid, as SQL.
    static let lacksTxid = SharedPaymentSQL.lacksTxid()

    /// An on-chain receive row, as SQL; [prefix] qualifies the columns (e.g. "p.").
    static func isReceiveRow(_ prefix: String = "") -> String {
        "\(prefix)payment_type = 'onchain' AND \(prefix)direction = 'received'"
    }

    /// The placeholder [row] lists a txid among [marks] as a candidate, as SQL.
    static func listsCandidate(_ row: String = "payments", among marks: String = "?") -> String {
        """
        EXISTS (
            SELECT 1 FROM onchain_deposit_candidates c WHERE c.payment_id = \(row).payment_id AND c.txid IN (\(marks))
        )
        """
    }

    /// An on-chain placeholder still pending since before the ?-bound cutoff, as SQL.
    static let isStalePlaceholder =
        "payment_type = 'onchain' AND status = 'pending' AND \(lacksTxid) AND created_at < ?"

    /// A receive row's address, as SQL over `payments p LEFT JOIN onchain_receive_txids r`: a
    /// placeholder's address lives on its resolution row, a txid row's on the payment.
    static let receiveRowAddress = "COALESCE(NULLIF(p.address, ''), r.address)"

    /// Whether `address` is one this wallet generated for receiving: the current receive address,
    /// or an older one it has received to before. A failed lookup counts as not ours.
    func isOwnReceiveAddress(_ address: String?, currentReceiveAddress: String?) -> Bool {
        guard let address, !address.isEmpty else { return false }
        if address == currentReceiveAddress { return true }
        return (try? isKnownReceiveAddress(address)) == true
    }

    /// True if this address has ever received an on-chain deposit we recorded, i.e. it is one of
    /// our own wallet's addresses rather than an external destination. A placeholder's address
    /// lives on its resolution row.
    private func isKnownReceiveAddress(_ address: String) throws -> Bool {
        let sql = """
        SELECT 1 FROM payments p
        LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
        WHERE \(Self.isReceiveRow("p.")) AND \(Self.receiveRowAddress) = ?
        LIMIT 1
        """
        return try !rawSQL.query(sql, params: [.text(address)]).isEmpty
    }

    /// The oldest pending placeholder for [address] (or [rowId] alone) that lists [txid] as a
    /// candidate and is at least [amountMsat], preferring one of exactly that amount.
    private func candidatePlaceholder(
        for txid: String,
        covering amountMsat: UInt64,
        address: String,
        rowId: Int64? = nil
    ) throws -> (id: Int64, amountMsat: Int64)? {
        guard let msat = Int64(exactly: amountMsat) else { return nil }
        return try rawSQL.query(
            """
            SELECT p.id, p.amount_msat FROM payments p
            LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
            WHERE \(Self.isReceiveRow("p.")) AND p.status = 'pending'
              AND \(SharedPaymentSQL.lacksTxid("p.txid")) AND p.amount_msat >= ?
              AND (r.address IS NULL OR r.address = ?) AND (? IS NULL OR p.id = ?)
              AND \(Self.listsCandidate("p"))
            ORDER BY p.amount_msat = ? DESC, p.id ASC LIMIT 1
            """,
            params: [
                .integer(msat), .text(address), rowId.map { .integer($0) } ?? .null,
                rowId.map { .integer($0) } ?? .null, .text(txid), .integer(msat)
            ]
        ).first.map { (id: $0.int64(0), amountMsat: $0.int64(1)) }
    }

    /// Writes [txid] on the placeholder [rowId] and resolves its resolution row, so the address
    /// resolver stops. The row stays pending under its own payment id: the confirmation poller
    /// completes it, and its candidates stay linked.
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
            "UPDATE payments SET txid = ?, address = ? WHERE id = ?",
            params: [.text(txid), .text(address), .integer(rowId)]
        )
    }

    /// Gives [amountMsat] of a merged placeholder up to the deposit that is split out of it.
    private func shrinkPlaceholder(rowId: Int64, byMsat amountMsat: Int64) throws {
        try rawSQL.execute(
            """
            UPDATE payments
            SET amount_usd = amount_usd * (amount_msat - ?) / amount_msat,
                amount_msat = amount_msat - ?
            WHERE id = ?
            """,
            params: [.integer(amountMsat), .integer(amountMsat), .integer(rowId)]
        )
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
            WHERE \(Self.isReceiveRow()) AND txid = ? AND amount_msat > 0
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
            WHERE \(Self.isReceiveRow()) AND status != 'failed'
              AND created_at >= ? AND txid IN (\(marks))
            """,
            params: [.integer(since)] + txids.sorted().map { .text($0) }
        ).reduce(into: [:]) { sats, row in
            if let txid = row.optString(0) { sats[txid] = row.int64(1) / 1000 }
        }
    }

    /// Records the txids that could be the placeholder deposit [paymentId]: the arrivals of its
    /// balance jump no recorded receive explained. Only these may later take the row over.
    func recordDepositCandidates(paymentId: String, txids: Set<String>) throws {
        for txid in txids.sorted() {
            try rawSQL.execute(
                "INSERT OR IGNORE INTO onchain_deposit_candidates (payment_id, txid) VALUES (?, ?)",
                params: [.text(paymentId), .text(txid)]
            )
        }
    }

    /// Completes pending placeholders created before [cutoff] that list a candidate txid in
    /// [walletTxids]: the wallet still holds a transaction the deposit can be, so it arrived. Returns
    /// how many rows were completed.
    @discardableResult
    func completeStalePlaceholders(createdBefore cutoff: Int64, walletTxids: Set<String>) throws -> Int {
        guard !walletTxids.isEmpty else { return 0 }
        let marks = Array(repeating: "?", count: walletTxids.count).joined(separator: ", ")
        return try rawSQL.executeReturningChanges(
            """
            UPDATE payments SET status = 'completed'
            WHERE \(Self.isStalePlaceholder) AND direction = 'received'
              AND \(Self.listsCandidate(among: marks))
            """,
            params: [.integer(cutoff)] + walletTxids.sorted().map { .text($0) }
        )
    }

    /// Fails placeholders still pending since before `cutoff`: nothing can ever confirm them. Sends
    /// are included, as on Android: a send without a txid never confirms either. Returns how many
    /// rows were failed.
    @discardableResult
    func failStalePlaceholders(createdBefore cutoff: Int64) throws -> Int {
        try rawSQL.executeReturningChanges(
            "UPDATE payments SET status = 'failed' WHERE \(Self.isStalePlaceholder)",
            params: [.integer(cutoff)]
        )
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
                WHERE \(Self.isReceiveRow())
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
                WHERE \(Self.isReceiveRow())
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

    /// The pending placeholder the address resolver [resolutionId] works for, with its address.
    func pendingPlaceholder(resolutionId: Int64) -> ReceiveTxidAdoption.Placeholder? {
        guard let row = try? rawSQL.query(
            """
            SELECT p.id, r.address, p.amount_msat FROM payments p
            JOIN onchain_receive_txids r ON r.id = p.resolution_id
            WHERE \(Self.isReceiveRow("p.")) AND p.status = 'pending'
              AND \(SharedPaymentSQL.lacksTxid("p.txid")) AND p.resolution_id = ?
            ORDER BY p.created_at ASC LIMIT 1
            """,
            params: [.integer(resolutionId)]
        ).first else { return nil }
        return ReceiveTxidAdoption.Placeholder(id: row.int64(0), address: row.string(1), amountMsat: row.int64(2))
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

    // MARK: - Wallet-list adoption

    /// Bounds the candidate scan; a wallet never has this many deposits pending at once.
    private static let maxPlaceholderCandidates = 256

    /// Pending placeholders that have an address, with the candidate txids recorded for them,
    /// oldest first. Rows without an address or candidates are never adopted.
    func placeholdersWithCandidates() -> [ReceiveTxidAdoption.Placeholder] {
        let rows = (try? rawSQL.query(
            """
            SELECT p.id, \(Self.receiveRowAddress) AS row_address, p.amount_msat, c.txid
            FROM payments p
            LEFT JOIN onchain_receive_txids r ON r.id = p.resolution_id
            JOIN onchain_deposit_candidates c ON c.payment_id = p.payment_id
            WHERE \(Self.isReceiveRow("p.")) AND p.status = 'pending'
              AND \(SharedPaymentSQL.lacksTxid("p.txid")) AND row_address IS NOT NULL AND row_address != ''
            ORDER BY p.created_at ASC, p.id ASC LIMIT \(Self.maxPlaceholderCandidates)
            """,
            params: []
        )) ?? []
        var placeholders: [ReceiveTxidAdoption.Placeholder] = []
        for row in rows {
            let id = row.int64(0)
            if placeholders.last?.id != id {
                placeholders.append(
                    ReceiveTxidAdoption.Placeholder(id: id, address: row.string(1), amountMsat: row.int64(2))
                )
            }
            placeholders[placeholders.count - 1].candidateTxids.insert(row.string(3))
        }
        return placeholders
    }
}
