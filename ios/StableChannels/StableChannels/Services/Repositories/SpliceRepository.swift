import Foundation
import SQLite3

struct PendingSpliceFailureCheck {
    let txid: String
    let channelId: String
    let userChannelId: String
}

final class SpliceRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    func deferFailureCheck(txid: String, channelId: String, userChannelId: String) throws {
        try rawSQL.execute(
            """
            INSERT INTO pending_splice_failure_checks (txid, channel_id, user_channel_id)
            VALUES (?, ?, ?) ON CONFLICT(txid) DO NOTHING
            """,
            params: [.text(txid), .text(channelId), .text(userChannelId)]
        )
    }

    func pendingFailureChecks() throws -> [PendingSpliceFailureCheck] {
        try rawSQL.query(
            "SELECT txid, channel_id, user_channel_id FROM pending_splice_failure_checks ORDER BY created_at, rowid"
        ).map { PendingSpliceFailureCheck(txid: $0.string(0), channelId: $0.string(1), userChannelId: $0.string(2)) }
    }

    func clearFailureCheck(txid: String) throws {
        try rawSQL.execute("DELETE FROM pending_splice_failure_checks WHERE txid = ?", params: [.text(txid)])
    }

    /// Never target the newest initiation row: an asynchronous check owns one exact txid.
    /// Retire its recovery obligation only if the final status write also commits.
    @discardableResult
    func failUnbroadcastSplice(txid: String) throws -> Bool {
        try rawSQL.inTransaction {
            let changed = try rawSQL.executeReturningChanges(
                """
                UPDATE payments SET status = 'failed'
                WHERE payment_type IN ('splice_in', 'splice_out') AND txid = ? AND status = 'pending'
                """,
                params: [.text(txid)]
            )
            try clearFailureCheck(txid: txid)
            return changed > 0
        }
    }

    /// Bounds one ready-event recovery pass; unresolved events stay for the next pass.
    static let readyEventRecoveryBatchSize = 100

    /// Persists the pending history row and the operation identity in one transaction, before the
    /// native splice call, so every later event can only be matched to this operation.
    @discardableResult
    func recordPendingSplice(
        paymentType: String,
        amountMsat: UInt64,
        amountUSD: Double?,
        btcPrice: Double?,
        address: String? = nil,
        channel: SpliceChannelIdentity
    ) throws -> Int64 {
        guard paymentType == "splice_in" || paymentType == "splice_out",
              !channel.userChannelId.isEmpty, !channel.channelId.isEmpty else {
            throw DatabaseError.executeFailed("Splice operation identity is incomplete")
        }
        return try rawSQL.inTransaction {
            try rawSQL.execute(
                """
                INSERT INTO payments (payment_type, direction, amount_msat, amount_usd, btc_price, status, address)
                VALUES (?, ?, ?, ?, ?, 'pending', ?)
                """,
                params: [
                    .text(paymentType), .text(paymentType == "splice_out" ? "sent" : "received"),
                    .integer(Int64(amountMsat)), amountUSD.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null, address.map { .text($0) } ?? .null
                ]
            )
            let rowId = rawSQL.lastInsertRowId
            try rawSQL.execute(
                """
                INSERT INTO splice_operations (payment_row_id, user_channel_id, channel_id, previous_funding_txid)
                VALUES (?, ?, ?, ?)
                """,
                params: [
                    .integer(rowId), .text(channel.userChannelId), .text(channel.channelId),
                    channel.previousFundingTxid.map { .text($0) } ?? .null
                ]
            )
            return rowId
        }
    }

    /// Commits the event receipt and any txid it lets an operation claim; once this returns the
    /// event is safe to acknowledge.
    func recordEvent(_ receipt: SpliceEventReceipt) throws {
        let params: [SQLValue] = [
            .text(receipt.eventKey), .text(receipt.kind.rawValue), .text(receipt.userChannelId),
            .text(receipt.channelId), receipt.fundingTxid.map { .text($0) } ?? .null,
            receipt.fundingVout.map { .integer(Int64($0)) } ?? .null
        ]
        try rawSQL.inTransaction {
            try rawSQL.execute(SpliceEventRecorder.insertEventSQL, params: params)
            try rawSQL.execute(SpliceEventRecorder.attachFundingSQL, params: params)
            try recoverAssignments()
        }
    }

    /// Resolves saved ready events that arrived without funding, against the live channel, and
    /// releases any whose funding turned out to be the operation's starting outpoint.
    func recoverReadyEvents(
        liveFunding: (_ userChannelId: String, _ channelId: String) -> (txid: String, vout: UInt32)?
    ) throws {
        try rawSQL.inTransaction {
            let events = try rawSQL.query(
                """
                SELECT event_key, user_channel_id, channel_id FROM splice_events
                WHERE event_type = 'ready' AND txid IS NULL ORDER BY rowid LIMIT ?
                """,
                params: [.integer(Int64(Self.readyEventRecoveryBatchSize))]
            )
            for event in events {
                guard let funding = liveFunding(event.string(1), event.string(2)) else { continue }
                try rawSQL.execute(
                    "UPDATE splice_events SET txid = ?, vout = ? WHERE event_key = ? AND txid IS NULL",
                    params: [.text(funding.txid), .integer(Int64(funding.vout)), .text(event.string(0))]
                )
            }
            try rawSQL.execute(
                """
                UPDATE splice_events SET payment_row_id = NULL
                WHERE event_type = 'ready' AND EXISTS (
                    SELECT 1 FROM splice_operations s WHERE s.payment_row_id = splice_events.payment_row_id
                      AND s.previous_funding_txid = splice_events.txid
                )
                """
            )
            try recoverAssignments()
        }
    }

    /// Retries txid assignment for captured operations (after a restart or a txid collision).
    func recoverAssignments() throws {
        try rawSQL.inTransaction {
            for sql in SpliceEventRecorder.assignmentStatements {
                try rawSQL.execute(sql)
            }
        }
    }

    /// Whether exactly one splice row carries this txid and it is still pending.
    func isPendingSplice(txid: String) throws -> Bool {
        let rows = try rawSQL.query(
            "SELECT status FROM payments WHERE payment_type IN ('splice_in', 'splice_out') AND txid = ? LIMIT 2",
            params: [.text(txid)]
        )
        return rows.count == 1 && rows[0].string(0) == "pending"
    }

    func getPendingSpliceTxid() throws -> String? {
        let rows = try rawSQL.query(
            "SELECT txid FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') AND txid IS NOT NULL ORDER BY id DESC LIMIT 1"
        )
        return rows.first?.optString(0)
    }

    func hasPendingSplice() throws -> Bool {
        // Recover first: a captured event proves negotiation, so its row must not expire.
        try recoverAssignments()
        // Sweep stale NULL-txid initiation rows to 'expired' — NOT 'failed':
        // no failure signal was observed, the negotiation is just overdue.
        // Expired rows no longer count as an active pending splice, but they
        // stay recoverable — a late spliceNegotiated can still stamp its txid
        // via recoverAssignments and return the row to 'pending'.
        let noTxidCutoff = Int64(Date().timeIntervalSince1970) - 600
        try rawSQL.execute(
            """
            UPDATE payments
            SET status = 'expired'
            WHERE status = 'pending'
              AND payment_type IN ('splice_in', 'splice_out')
              AND txid IS NULL
              AND created_at < ?
            """,
            params: [.integer(noTxidCutoff)]
        )
        let rows = try rawSQL.query(SpliceEventRecorder.hasPendingSpliceSQL)
        return !rows.isEmpty
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

    /// Marks the latest pre-negotiation (NULL-txid) splice row failed. This is
    /// an explicit failure signal (native call threw, or spliceNegotiationFailed
    /// arrived), so it also finalizes an `expired` row: once the failure is
    /// known the row must become terminal instead of staying recoverable.
    /// Rows that already carry a txid are left alone: their tx may be on-chain
    /// and the confirmation monitor still owns them, so a failed native call or
    /// a stale negotiation-failure replay must not tear them down.
    @discardableResult
    func failLatestPendingSplice() -> Bool {
        do {
            try rawSQL.execute(
                """
                UPDATE payments
                SET status = 'failed'
                WHERE id = (
                    SELECT id FROM payments
                    WHERE payment_type IN ('splice_in', 'splice_out')
                      AND status IN ('pending', 'expired')
                      AND txid IS NULL
                    ORDER BY id DESC LIMIT 1
                )
                """
            )
            return true
        } catch {
            return false
        }
    }
}
