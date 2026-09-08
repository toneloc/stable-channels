import Foundation
import SQLite3

final class SpliceRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    /// Stamps a negotiated txid onto the latest pending NULL-txid splice
    /// initiation row (either direction). Only `status = 'pending'` rows are
    /// candidates — failed rows are terminal and are never stamped or
    /// resurrected — and a txid already carried by any payment row is never
    /// assigned a second time (replayed spliceNegotiated events are no-ops).
    /// Note: `UPDATE ... ORDER BY ... LIMIT` is not supported by the SQLite
    /// build shipped on iOS, so the latest-row selection uses a subquery.
    func setPendingSpliceTxid(_ txid: String) throws {
        try rawSQL.execute(
            """
            UPDATE payments
            SET txid = ?
            WHERE id = (
                SELECT id FROM payments
                WHERE payment_type IN ('splice_in', 'splice_out')
                  AND status = 'pending'
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

    /// Marks the latest pre-negotiation (NULL-txid) pending splice row failed.
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
                      AND status = 'pending'
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
