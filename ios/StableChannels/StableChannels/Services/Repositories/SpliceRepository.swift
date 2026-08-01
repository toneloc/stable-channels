import Foundation
import SQLite3

final class SpliceRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
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
}
