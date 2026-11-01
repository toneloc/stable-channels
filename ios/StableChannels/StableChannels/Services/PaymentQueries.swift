import Foundation

/// Read-only payment and trade lookups for views. Writes stay with the services and AppState
/// commands that own them.
@MainActor
struct PaymentQueries {
    let databaseService: DatabaseService?

    func pendingOnchainActivity() -> (receives: [PaymentRecord], sends: [PaymentRecord]) {
        guard let repo = databaseService?.paymentRepo else { return ([], []) }
        return (repo.pendingOnchainReceives(), repo.pendingOnchainSends())
    }

    func latestReceivedPayment() -> PaymentRecord? {
        databaseService?.paymentRepo.latestReceivedPayment()
    }

    /// nil when there is no database yet; throws when the lookup fails.
    func payment(byId id: Int64) throws -> PaymentRecord? {
        try databaseService?.paymentRepo.getPayment(byId: id)
    }

    func payment(paymentId: String) -> PaymentRecord? {
        databaseService?.paymentRepo.payment(paymentId: paymentId)
    }

    func payment(txid: String) -> PaymentRecord? {
        databaseService?.paymentRepo.payment(txid: txid)
    }

    /// Throws when there is no database yet or the lookup fails.
    func recentTrades(limit: Int) throws -> [TradeRecord] {
        guard let databaseService else { throw DatabaseError.openFailed("Database unavailable") }
        return try databaseService.channelRepo.getRecentTrades(limit: limit)
    }

    /// Throws when there is no database yet or the lookup fails.
    func recentPayments(limit: Int) throws -> [PaymentRecord] {
        guard let databaseService else { throw DatabaseError.openFailed("Database unavailable") }
        return try databaseService.paymentRepo.getRecentPayments(limit: limit)
    }
}
