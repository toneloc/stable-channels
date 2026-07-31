import Foundation
import SQLite3

final class PendingOperationRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    @discardableResult
    func insertPendingOperation(
        opId: String,
        opType: String,
        fundingOutpointTxid: String?,
        fundingOutpointVout: UInt32?,
        balanceSats: UInt64? = nil,
        balanceUsd: Double? = nil,
        btcPrice: Double? = nil,
        counterparty: String? = nil
    ) -> Bool {
        do {
            try rawSQL.execute(
                """
                INSERT INTO pending_operations
                    (op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                     balance_sats, balance_usd, btc_price, counterparty, status)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending')
                ON CONFLICT(op_id) DO UPDATE SET
                    op_type = excluded.op_type,
                    funding_outpoint_txid = excluded.funding_outpoint_txid,
                    funding_outpoint_vout = excluded.funding_outpoint_vout,
                    balance_sats = excluded.balance_sats,
                    balance_usd = excluded.balance_usd,
                    btc_price = excluded.btc_price,
                    counterparty = excluded.counterparty,
                    status = 'pending'
                """,
                params: [
                    .text(opId),
                    .text(opType),
                    fundingOutpointTxid.map { .text($0) } ?? .null,
                    fundingOutpointVout.map { .integer(Int64($0)) } ?? .null,
                    balanceSats.map { .integer(Int64($0)) } ?? .null,
                    balanceUsd.map { .real($0) } ?? .null,
                    btcPrice.map { .real($0) } ?? .null,
                    counterparty.map { .text($0) } ?? .null
                ]
            )
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    func updatePendingOperation(opId: String, closingTxid: String, status: String) -> Bool {
        do {
            try rawSQL.execute(
                """
                UPDATE pending_operations
                SET closing_txid = ?, status = ?, resolved_at = strftime('%s', 'now')
                WHERE op_id = ? AND status = 'pending'
                """,
                params: [.text(closingTxid), .text(status), .text(opId)]
            )
            return rawSQL.changes > 0
        } catch {
            return false
        }
    }

    func fetchPendingOperations() -> [PendingOperation] {
        do {
            let rows = try rawSQL.query(
                """
                SELECT op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                       closing_txid, balance_sats, balance_usd, btc_price, counterparty,
                       status, created_at, resolved_at
                FROM pending_operations
                WHERE status = 'pending'
                """
            )
            return rows.map { Self.parsePendingOperation($0) }
        } catch {
            return []
        }
    }

    func fetchPendingOperation(opId: String) -> PendingOperation? {
        do {
            let rows = try rawSQL.query(
                """
                SELECT op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                       closing_txid, balance_sats, balance_usd, btc_price, counterparty,
                       status, created_at, resolved_at
                FROM pending_operations
                WHERE op_id = ?
                LIMIT 1
                """,
                params: [.text(opId)]
            )
            return rows.first.map { Self.parsePendingOperation($0) }
        } catch {
            return nil
        }
    }

    func fetchPendingOperationByFundingTxid(_ txid: String) -> PendingOperation? {
        do {
            let rows = try rawSQL.query(
                """
                SELECT op_id, op_type, funding_outpoint_txid, funding_outpoint_vout,
                       closing_txid, balance_sats, balance_usd, btc_price, counterparty,
                       status, created_at, resolved_at
                FROM pending_operations
                WHERE funding_outpoint_txid = ? AND status = 'pending'
                LIMIT 1
                """,
                params: [.text(txid)]
            )
            return rows.first.map { Self.parsePendingOperation($0) }
        } catch {
            return nil
        }
    }

    private static func parsePendingOperation(_ row: [Any?]) -> PendingOperation {
        PendingOperation(
            opId: row.string(0),
            opType: row.string(1),
            fundingOutpointTxid: row.optString(2),
            fundingOutpointVout: row.optUInt32(3),
            closingTxid: row.optString(4),
            balanceSats: row.optUInt64(5),
            balanceUsd: row.optDouble(6),
            btcPrice: row.optDouble(7),
            counterparty: row.optString(8),
            status: row.string(9, default: "pending"),
            createdAt: row.int64(10),
            resolvedAt: row.optInt64(11)
        )
    }
}
