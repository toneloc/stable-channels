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
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            if let pid = paymentId, !pid.isEmpty {
                let existing = try rawSQL.query(
                    "SELECT id FROM payments WHERE payment_id = ?",
                    params: [.text(pid)]
                )
                if !existing.isEmpty {
                    try backfillPaymentDetails(
                        paymentId: pid,
                        paymentType: paymentType,
                        direction: direction,
                        amountMsat: amountMsat,
                        amountUSD: amountUSD,
                        btcPrice: btcPrice,
                        counterparty: counterparty,
                        address: address,
                        txid: txid,
                        clearPlaceholder: true
                    )
                    return false
                }
            }

            // Wrapped in inTransaction to ensure existence check, insert, and backfill are atomic.
            let sql = """
                INSERT OR IGNORE INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, txid, address)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """
            let inserted = try rawSQL.executeReturningChanges(sql, params: [
                paymentId.map { .text($0) } ?? .null,
                .text(paymentType), .text(direction), .integer(Int64(amountMsat)),
                amountUSD.map { .real($0) } ?? .null,
                btcPrice.map { .real($0) } ?? .null,
                counterparty.map { .text($0) } ?? .null,
                .text(status),
                txid.map { .text($0) } ?? .null,
                address.map { .text($0) } ?? .null
            ])
            if inserted == 0, let pid = paymentId, !pid.isEmpty {
                try backfillPaymentDetails(
                    paymentId: pid,
                    paymentType: paymentType,
                    direction: direction,
                    amountMsat: amountMsat,
                    amountUSD: amountUSD,
                    btcPrice: btcPrice,
                    counterparty: counterparty,
                    address: address,
                    txid: txid,
                    clearPlaceholder: true
                )
            }
            return inserted > 0
        }
    }

    func updatePaymentStatus(paymentId: String, status: String, feeMsat: UInt64? = nil) throws {
        do {
            try rawSQL.inTransaction(mode: "IMMEDIATE") {
                let updated: Int
                if let fee = feeMsat {
                    updated = try rawSQL.executeReturningChanges(
                        SharedPaymentSQL.resolvePendingPaymentWithFee,
                        params: [.text(status), .integer(Int64(fee)), .text(paymentId)]
                    )
                } else {
                    updated = try rawSQL.executeReturningChanges(
                        SharedPaymentSQL.resolvePendingPayment,
                        params: [.text(status), .text(paymentId)]
                    )
                }

                if updated == 0 {
                    let existingRows = try rawSQL.query(
                        "SELECT id, status, fee_msat FROM payments WHERE payment_id = ? ORDER BY id DESC LIMIT 1",
                        params: [.text(paymentId)]
                    )
                    if let existing = existingRows.first {
                        let existingStatus = existing.string(1)
                        let existingFee = UInt64(existing.int64(2))
                        // Terminal status is immutable; only backfill fee if incoming event matches existing status and
                        // existing fee is zero.
                        if existingStatus == status, let fee = feeMsat, existingFee == 0 {
                            try rawSQL.execute(
                                "UPDATE payments SET fee_msat = ? WHERE payment_id = ?",
                                params: [.integer(Int64(fee)), .text(paymentId)]
                            )
                        }
                    } else {
                        // Settle or fail event arrived before recordPayment() inserted the row.
                        // Insert placeholder with terminal status so recordPayment won't downgrade it.
                        let sql = """
                            INSERT OR IGNORE INTO payments (payment_id, payment_type, direction, amount_msat, status, fee_msat, is_placeholder)
                            VALUES (?, 'lightning', 'sent', 0, ?, ?, 1)
                        """
                        try rawSQL.execute(
                            sql,
                            params: [
                                .text(paymentId),
                                .text(status),
                                .integer(Int64(feeMsat ?? 0))
                            ]
                        )
                    }
                }
            }
        } catch {
            AuditService.log("UPDATE_PAYMENT_STATUS_FAILED", data: [
                "payment_id": paymentId,
                "status": status,
                "fee_msat": feeMsat.map { "\($0)" } ?? "nil",
                "error": error.localizedDescription
            ])
            throw error
        }
    }

    /// Records our own outbound payment as pending, unless its event handler already settled or
    /// failed it: then only the details are backfilled, never a downgrade back to pending.
    func recordOutgoingPending(
        paymentId: String,
        paymentType: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        address: String?,
        txid: String?
    ) throws {
        if let existing = payment(paymentId: paymentId), existing.isSettled || existing.isFailed {
            try backfillPaymentDetails(
                paymentId: paymentId, amountMsat: amountMsat, amountUSD: amountUSD, btcPrice: btcPrice,
                counterparty: nil, address: address, txid: txid
            )
            return
        }
        _ = try recordPayment(
            paymentId: paymentId, paymentType: paymentType, direction: "sent", amountMsat: amountMsat,
            amountUSD: amountUSD, btcPrice: btcPrice, counterparty: nil, status: "pending",
            txid: txid, address: address
        )
    }

    func backfillPaymentDetails(
        paymentId: String,
        paymentType: String? = nil,
        direction: String? = nil,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        counterparty: String?,
        address: String? = nil,
        txid: String? = nil,
        clearPlaceholder: Bool = true
    ) throws {
        let sql = """
            UPDATE payments
            SET payment_type = CASE WHEN is_placeholder = 1 THEN COALESCE(?, payment_type) ELSE payment_type END,
                direction = CASE WHEN is_placeholder = 1 THEN COALESCE(?, direction) ELSE direction END,
                amount_msat = CASE WHEN is_placeholder = 1 OR amount_msat = 0 THEN ? ELSE amount_msat END,
                amount_usd = COALESCE(amount_usd, ?),
                btc_price = COALESCE(btc_price, ?),
                counterparty = COALESCE(counterparty, ?),
                address = COALESCE(address, ?),
                txid = COALESCE(txid, ?),
                is_placeholder = CASE WHEN ? = 1 THEN 0 ELSE is_placeholder END
            WHERE payment_id = ?
        """
        try rawSQL.execute(sql, params: [
            paymentType.map { .text($0) } ?? .null,
            direction.map { .text($0) } ?? .null,
            .integer(Int64(amountMsat)),
            amountUSD.map { .real($0) } ?? .null,
            btcPrice.map { .real($0) } ?? .null,
            counterparty.map { .text($0) } ?? .null,
            address.map { .text($0) } ?? .null,
            txid.map { .text($0) } ?? .null,
            .integer(clearPlaceholder ? 1 : 0),
            .text(paymentId)
        ])
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

    /// Fails txid-less onchain rows still pending since before `cutoff`: nothing can ever
    /// confirm them. Returns how many rows were failed.
    @discardableResult
    func failStaleTxidlessOnchain(createdBefore cutoff: Int64) throws -> Int {
        try rawSQL.executeReturningChanges(
            """
            UPDATE payments SET status = 'failed'
            WHERE payment_type = 'onchain' AND status = 'pending'
            AND (txid IS NULL OR txid = '') AND created_at < ?
            """,
            params: [.integer(cutoff)]
        )
    }

    /// Fails one pending, unconfirmed onchain row. The caller decides it is stale first
    /// (`ConfirmationPolicy.shouldFailUnfound`); the WHERE guard only keeps a row that was
    /// confirmed in the meantime.
    func failUnconfirmedOnchainRow(id: Int64) throws -> Bool {
        try rawSQL.executeReturningChanges(
            """
            UPDATE payments SET status = 'failed'
            WHERE id = ? AND payment_type = 'onchain' AND status = 'pending' AND confirmations = 0
            """,
            params: [.integer(id)]
        ) > 0
    }

    /// Detaches a txid that does not pay the row's address, back to an unresolved pending row.
    func clearPaymentTxid(id: Int64) throws -> Bool {
        try rawSQL.executeReturningChanges(
            """
            UPDATE payments SET txid = NULL, confirmations = 0, tx_block_height = NULL, status = 'pending'
            WHERE id = ?
            """,
            params: [.integer(id)]
        ) > 0
    }

    func latestReceivedPayment() -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE direction = "received"
        AND is_placeholder = 0
        AND NOT (payment_type = 'lightning' AND amount_msat < 1000)
        ORDER BY id DESC LIMIT 1
        """
        guard let row = try? rawSQL.query(sql, params: []).first else { return nil }
        return paymentRecord(from: row)
    }

    func payment(paymentId: String, excludePlaceholders: Bool = false) -> PaymentRecord? {
        let placeholderClause = excludePlaceholders ? "AND is_placeholder = 0" : ""
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE payment_id = ? \(placeholderClause)
        ORDER BY id DESC LIMIT 1
        """
        guard let row = try? rawSQL.query(sql, params: [.text(paymentId)]).first else { return nil }
        return paymentRecord(from: row)
    }

    func payment(txid: String) -> PaymentRecord? {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE txid = ?
        ORDER BY id DESC LIMIT 1
        """
        guard let row = try? rawSQL.query(sql, params: [.text(txid)]).first else { return nil }
        return paymentRecord(from: row)
    }

    /// Returns true if an outgoing payment is currently in-flight ('pending').
    func hasPendingOutgoingPayment() throws -> Bool {
        let sql = """
            SELECT 1 FROM payments
            WHERE direction = 'sent' AND status = 'pending'
            LIMIT 1
        """
        let rows = try rawSQL.query(sql, params: [])
        return !rows.isEmpty
    }

    /// Outbound Lightning rows still pending, oldest first, for the foreground recovery sweep.
    func pendingOutgoingLightningPaymentIds(limit: Int = LightningPaymentRecovery.reconcileLimit) throws -> [String] {
        try rawSQL.query(
            """
            SELECT payment_id FROM payments
            WHERE payment_id IS NOT NULL AND payment_id != ''
              AND payment_type IN ('lightning', 'bolt12')
              AND direction = 'sent' AND status = 'pending'
            ORDER BY created_at ASC LIMIT ?
            """,
            params: [.integer(Int64(limit))]
        ).map { $0.string(0) }
    }

    static let pendingOnchainReceivesLimit = 25
    static let pendingOnchainSendsLimit = 10

    /// Onchain deposits still confirming: the newest `limit`, returned oldest first. Home shows one "+" row each.
    func pendingOnchainReceives(limit: Int = pendingOnchainReceivesLimit) -> [PaymentRecord] {
        pendingOnchainRows(direction: "received", limit: limit)
    }

    /// Onchain sends still confirming: the newest `limit`, returned oldest first. Home shows one "-" row each.
    func pendingOnchainSends(limit: Int = pendingOnchainSendsLimit) -> [PaymentRecord] {
        pendingOnchainRows(direction: "sent", limit: limit)
    }

    private func pendingOnchainRows(direction: String, limit: Int) -> [PaymentRecord] {
        let sql = """
        SELECT * FROM (
            SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
            counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
            FROM payments
            WHERE payment_type = 'onchain'
            AND direction = ?
            AND status = 'pending'
            ORDER BY created_at DESC, id DESC
            LIMIT ?
        ) ORDER BY created_at ASC, id ASC
        """
        let rows = (try? rawSQL.query(sql, params: [.text(direction), .integer(Int64(limit))])) ?? []
        return rows.map { paymentRecord(from: $0) }
    }

    /// Rows the confirmation poller advances. Splices are deliberately NOT here.
    /// A splice row's completion triggers stable-books reconcile in AppState.completeConfirmedSplice().
    /// Excluding splices ensures the poller does not race the monitor and complete rows without deducting books.
    func paymentsNeedingConfirmation() throws -> [PaymentRecord] {
        let defaultRequired = ConfirmationPolicy.defaultRequiredConfirmations
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
        counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
        FROM payments
        WHERE txid IS NOT NULL
        AND txid != ''
        AND status != 'failed'
        AND payment_type IN ('onchain', 'channel_close')
        AND (confirmations IS NULL OR confirmations < ?)
        ORDER BY created_at DESC
        LIMIT 50
        """
        let rows = try rawSQL.query(
            sql,
            params: [.integer(Int64(defaultRequired))]
        )
        return rows.map { row in
            paymentRecord(from: row)
        }
    }

    /// Check if any payment row exists carrying this transaction ID.
    func paymentExists(forTxid txid: String) throws -> Bool {
        let sql = "SELECT 1 FROM payments WHERE txid = ? LIMIT 1"
        let rows = try rawSQL.query(sql, params: [.text(txid)])
        return !rows.isEmpty
    }

    /// True if this address has ever received an on-chain deposit we recorded, i.e. it is one of
    /// our own wallet's addresses rather than an external destination.
    /// Whether the newest row carrying `txid` is still pending.
    func hasPendingPayment(txid: String) -> Bool {
        payment(txid: txid)?.status == "pending"
    }

    /// Whether `address` is one this wallet generated for receiving: the current receive address,
    /// or an older one it has received to before. A failed lookup counts as not ours.
    func isOwnReceiveAddress(_ address: String?, currentReceiveAddress: String?) -> Bool {
        guard let address, !address.isEmpty else { return false }
        if address == currentReceiveAddress { return true }
        return (try? isKnownReceiveAddress(address)) == true
    }

    func isKnownReceiveAddress(_ address: String) throws -> Bool {
        let sql = """
        SELECT 1 FROM payments
        WHERE payment_type = 'onchain' AND direction = 'received' AND address = ?
        LIMIT 1
        """
        return try !rawSQL.query(sql, params: [.text(address)]).isEmpty
    }

    /// Finds the payment_id of a matching unconsumed channel_close payment within mining fee tolerance (Query).
    func findMatchingChannelClosePaymentId(depositSats: UInt64, withinSecs: Int64 = 86400) -> String? {
        let sql = """
        SELECT id, payment_id, amount_msat FROM payments
        WHERE payment_type = 'channel_close'
        AND direction = 'received'
        AND created_at >= ?
        ORDER BY created_at DESC
        LIMIT 5
        """
        let cutoff = Int64(Date().timeIntervalSince1970) - withinSecs
        guard let rows = try? rawSQL.query(sql, params: [.integer(cutoff)]) else { return nil }
        for row in rows {
            let rowId = row.int64(0)
            let paymentId = row.string(1) ?? "\(rowId)"
            let msat = row.int64(2)
            guard msat > 0 else { continue }

            let checkSql = "SELECT 1 FROM consumed_close_sweeps WHERE payment_id = ? LIMIT 1"
            if let consumedRows = try? rawSQL.query(checkSql, params: [.text(paymentId)]), !consumedRows.isEmpty {
                continue
            }

            let closeSats = UInt64(msat / 1000)
            let matched: Bool
            if depositSats <= closeSats && (closeSats - depositSats) <= 15000 {
                matched = true
            } else if depositSats >= closeSats && (depositSats - closeSats) <= 15000 {
                matched = true
            } else {
                matched = false
            }

            if matched {
                return paymentId
            }
        }
        return nil
    }

    /// Marks a close sweep payment as consumed so subsequent deposits are not swallowed (Command).
    @discardableResult
    func markCloseSweepConsumed(paymentId: String) -> Bool {
        let sql = "INSERT OR IGNORE INTO consumed_close_sweeps (payment_id) VALUES (?)"
        do {
            try rawSQL.execute(sql, params: [.text(paymentId)])
            return true
        } catch {
            return false
        }
    }

    /// True if a channel_close payment row exists matching this deposit amount within mining fee tolerance,
    /// preventing close sweeps from creating duplicate on-chain deposit rows.
    /// When `consume` is true, also marks the matching payment consumed so subsequent deposits are not swallowed.
    func hasMatchingChannelClosePayment(depositSats: UInt64, withinSecs: Int64 = 86400, consume: Bool = false) -> Bool {
        guard let paymentId = findMatchingChannelClosePaymentId(depositSats: depositSats, withinSecs: withinSecs) else {
            return false
        }
        if consume {
            markCloseSweepConsumed(paymentId: paymentId)
        }
        return true
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

    func updateConfirmations(
        paymentId: Int64,
        paymentType: String? = nil,
        txBlockHeight: UInt32,
        currentBlockHeight: UInt32,
        calculator: ConfirmationCalculating = ConfirmationCalculator()
    ) throws {
        let type: String
        if let paymentType {
            type = paymentType
        } else {
            type = try getPayment(byId: paymentId)?.paymentType ?? "onchain"
        }
        let required = ConfirmationPolicy.requiredConfirmations(for: type)
        let progress = calculator.progress(
            for: txBlockHeight,
            currentBlockHeight: currentBlockHeight,
            required: required
        )
        let status = progress.isComplete ? "completed" : "pending"
        try rawSQL.execute(
            "UPDATE payments SET confirmations = ?, tx_block_height = ?, status = ? WHERE id = ?",
            params: [
                .integer(Int64(progress.display)),
                .integer(Int64(txBlockHeight)),
                .text(status),
                .integer(paymentId)
            ]
        )
    }

    func getRecentPayments(limit: Int) throws -> [PaymentRecord] {
        let sql = """
            SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
                   counterparty, status, created_at, fee_msat, txid, address, confirmations, tx_block_height
            FROM payments
            WHERE is_placeholder = 0
            AND NOT (payment_type = 'lightning' AND amount_msat < 1000)
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

    // MARK: - Seen Settlement IDs (STABILITY_PAYMENT_V1 replay protection)

    func isSettlementSeen(settlementId: String) -> Bool {
        do {
            let rows = try rawSQL.query(
                "SELECT 1 FROM seen_stability_settlements WHERE settlement_id = ? LIMIT 1",
                params: [.text(settlementId)]
            )
            return !rows.isEmpty
        } catch {
            return false
        }
    }

    private func burnSettlement(settlementId: String) throws {
        try rawSQL.execute(
            "INSERT INTO seen_stability_settlements (settlement_id) VALUES (?)",
            params: [.text(settlementId)]
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
        backingDeltaSats: Int64?,
        settlementId: String? = nil
    ) throws -> PaymentPersistenceResult {
        try rawSQL.inTransaction(mode: "IMMEDIATE") {
            // Replay guard inside the transaction that credits backing, so a crash can never leave
            // backing credited with the settlement id still replayable.
            if let sid = settlementId {
                let seen = try rawSQL.query(
                    "SELECT 1 FROM seen_stability_settlements WHERE settlement_id = ? LIMIT 1",
                    params: [.text(sid)]
                )
                if !seen.isEmpty {
                    let backing = try authoritativeBacking(
                        userChannelId: userChannelId,
                        required: backingDeltaSats != nil
                    )
                    return PaymentPersistenceResult(isNewPayment: false, backingSats: backing)
                }
            }

            if let pid = paymentId, !pid.isEmpty {
                let existing = try rawSQL.query(
                    "SELECT id, is_placeholder, backing_applied, payment_type FROM payments WHERE payment_id = ?",
                    params: [.text(pid)]
                )
                if let firstRow = existing.first {
                    let isPlaceholder = (firstRow.int64(1)) == 1
                    let alreadyApplied = (firstRow.int64(2)) != 0
                    let existingType = firstRow.string(3) ?? ""

                    var resultingBacking: UInt64?
                    let isOwedDebit = (isPlaceholder || existingType == "stability") && !alreadyApplied
                    if let delta = backingDeltaSats, isOwedDebit {
                        guard let ucid = userChannelId, !ucid.isEmpty else {
                            throw DatabaseError.executeFailed("userChannelId required for backing update")
                        }
                        let newBacking = try applyBacking(
                            userChannelId: ucid,
                            deltaSats: delta,
                            paymentType: paymentType,
                            direction: direction,
                            btcPrice: btcPrice
                        )
                        try rawSQL.execute(
                            "UPDATE payments SET backing_applied = 1 WHERE payment_id = ?",
                            params: [.text(pid)]
                        )
                        if let sid = settlementId {
                            try burnSettlement(settlementId: sid)
                        }
                        resultingBacking = UInt64(newBacking)
                    } else if backingDeltaSats != nil {
                        resultingBacking = try authoritativeBacking(
                            userChannelId: userChannelId,
                            required: true
                        )
                    }

                    try backfillPaymentDetails(
                        paymentId: pid,
                        paymentType: paymentType,
                        direction: direction,
                        amountMsat: amountMsat,
                        amountUSD: amountUSD,
                        btcPrice: btcPrice,
                        counterparty: nil,
                        address: nil,
                        txid: nil,
                        clearPlaceholder: true
                    )
                    return PaymentPersistenceResult(isNewPayment: false, backingSats: resultingBacking)
                }
            }

            let backingAppliedValue: Int64 = (backingDeltaSats != nil) ? 1 : 0
            try rawSQL.execute(
                "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status, backing_applied) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params: [
                    paymentId.map { .text($0) } ?? .null,
                    .text(paymentType), .text(direction), .integer(Int64(amountMsat)),
                    amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    .text(status),
                    .integer(backingAppliedValue)
                ]
            )
            if let sid = settlementId {
                try burnSettlement(settlementId: sid)
            }
            var resultingBacking: UInt64?
            if let delta = backingDeltaSats {
                guard let ucid = userChannelId, !ucid.isEmpty else {
                    throw DatabaseError.executeFailed("userChannelId required for backing update")
                }
                let newBacking = try applyBacking(
                    userChannelId: ucid,
                    deltaSats: delta,
                    paymentType: paymentType,
                    direction: direction,
                    btcPrice: btcPrice
                )
                resultingBacking = UInt64(newBacking)
            }
            return PaymentPersistenceResult(
                isNewPayment: true,
                backingSats: resultingBacking
            )
        }
    }

    /// Write the backing a payment row implies. Reads both books under the caller's write lock
    /// so an incoming stability receipt credits only the local shortfall (#320). Must run inside
    /// the caller's transaction.
    private func applyBacking(
        userChannelId ucid: String,
        deltaSats delta: Int64,
        paymentType: String,
        direction: String,
        btcPrice: Double?
    ) throws -> Int64 {
        let rows = try rawSQL.query(
            "SELECT stable_sats, expected_usd FROM channels WHERE user_channel_id = ?",
            params: [.text(ucid)]
        )
        guard let row = rows.first else {
            throw DatabaseError.missingChannelRow(ucid)
        }
        let current = row.int64(0)
        guard let newBacking = StabilityAllocation.backingAfterPayment(
            currentBackingSats: current,
            expectedUSD: row.double(1),
            price: btcPrice,
            deltaSats: delta,
            paymentType: paymentType,
            direction: direction
        ) else {
            throw DatabaseError.executeFailed("Incoming stability allocation unavailable")
        }
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
        return newBacking
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

    func recentConfirmedPayments(confirmedAfterHeight: UInt32) throws -> [PaymentRecord] {
        let sql = """
        SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd,
               btc_price, counterparty, status, created_at, fee_msat, txid, address,
               confirmations, tx_block_height
        FROM payments
        WHERE (txid IS NOT NULL OR address IS NOT NULL)
          AND status = 'completed'
          AND tx_block_height IS NOT NULL
          AND tx_block_height >= ?
        ORDER BY tx_block_height DESC;
        """
        let rows = try rawSQL.query(sql, params: [.integer(Int64(confirmedAfterHeight))])
        return rows.map { row in
            paymentRecord(from: row)
        }
    }

    func downgradePaymentToPending(paymentId: Int64) throws {
        let sql = """
        UPDATE payments
        SET status = 'pending', confirmations = 0, tx_block_height = NULL
        WHERE id = ?;
        """
        try rawSQL.execute(sql, params: [.integer(paymentId)])
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
}
