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

    /// Stamps a negotiated txid onto the latest NULL-txid splice initiation
    /// row (either direction). Candidates are `pending` rows plus `expired`
    /// rows (swept by the no-txid timeout in `hasPendingSplice`): mainnet
    /// negotiation can outlast that window, and a late spliceNegotiated must
    /// still find its initiation row — stamping moves the row back to
    /// `pending`. Explicitly `failed` rows (native call threw, or negotiation
    /// failed) are terminal and are never stamped or resurrected. A txid
    /// already carried by any payment row is never assigned a second time
    /// (replayed spliceNegotiated events are no-ops).
    /// Note: the latest-row selection uses an id subquery rather than
    /// `UPDATE ... ORDER BY ... LIMIT` for portability across SQLite builds
    /// compiled without ENABLE_UPDATE_DELETE_LIMIT.
    func setPendingSpliceTxid(_ txid: String) throws {
        try rawSQL.execute(
            """
            UPDATE payments
            SET txid = ?, status = 'pending'
            WHERE id = (
                SELECT id FROM payments
                WHERE payment_type IN ('splice_in', 'splice_out')
                  AND status IN ('pending', 'expired')
                  AND txid IS NULL
                ORDER BY id DESC LIMIT 1
            )
              AND NOT EXISTS (SELECT 1 FROM payments WHERE txid = ?)
            """,
            params: [.text(txid), .text(txid)]
        )
    }

    func getPendingSpliceTxid() throws -> String? {
        let rows = try rawSQL.query(
            "SELECT txid FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') AND txid IS NOT NULL ORDER BY id DESC LIMIT 1"
        )
        return rows.first?.optString(0)
    }

    func hasPendingSplice() throws -> Bool {
        // Sweep stale NULL-txid initiation rows to 'expired' — NOT 'failed':
        // no failure signal was observed, the negotiation is just overdue.
        // Expired rows no longer count as an active pending splice, but they
        // stay recoverable — a late spliceNegotiated can still stamp its txid
        // via setPendingSpliceTxid and return the row to 'pending'.
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
        let rows = try rawSQL.query(
            "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') LIMIT 1"
        )
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
