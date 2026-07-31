import Foundation
import SQLite3

final class StabilitySendRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    func claimPendingSend(amountMsat: UInt64, price: Double) -> Bool {
        do {
            return try rawSQL.inTransaction(mode: "IMMEDIATE") {
                let existing = try rawSQL.query("SELECT id FROM pending_stability_send WHERE id = 1")
                guard existing.isEmpty else { return false }
                try rawSQL.execute(
                    "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)",
                    params: [
                        .integer(Int64(amountMsat)),
                        .real(price),
                        .integer(Int64(Date().timeIntervalSince1970))
                    ]
                )
                return true
            }
        } catch {
            return false
        }
    }

    @discardableResult
    func setPendingSendPaymentId(_ paymentId: String) -> Bool {
        do {
            try rawSQL.execute(
                "UPDATE pending_stability_send SET payment_id = ? WHERE id = 1",
                params: [.text(paymentId)]
            )
            return true
        } catch {
            return false
        }
    }

    func loadPendingSend() -> PendingStabilitySend? {
        guard let rows = try? rawSQL.query(
            "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1"
        ), let row = rows.first else {
            return nil
        }
        return PendingStabilitySend(
            paymentId: row.string(0),
            amountMsat: row.uInt64(1),
            price: row.double(2),
            createdAt: row.int64(3)
        )
    }

    func clearPendingSend() {
        try? rawSQL.execute("DELETE FROM pending_stability_send WHERE id = 1")
    }

    func isOutgoingStabilityPayment(paymentId: String) throws -> Bool {
        let rows = try rawSQL.query(
            "SELECT 1 FROM payments WHERE payment_id = ? AND payment_type = 'stability' AND direction = 'sent' LIMIT 1",
            params: [.text(paymentId)]
        )
        return !rows.isEmpty
    }
}
