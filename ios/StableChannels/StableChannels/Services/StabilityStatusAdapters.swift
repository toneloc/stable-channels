import Foundation

/// Adapts LDK Node and SQLite database checks to PaymentStatusProviding.
final class LDKPaymentStatusAdapter: PaymentStatusProviding {
    private let nodeService: any NodeServiceProtocol
    private let databaseService: DatabaseService
    private let hasPendingNodePaymentCheck: () -> Bool

    init(
        nodeService: any NodeServiceProtocol,
        databaseService: DatabaseService,
        hasPendingNodePaymentCheck: (() -> Bool)? = nil
    ) {
        self.nodeService = nodeService
        self.databaseService = databaseService
        self.hasPendingNodePaymentCheck = hasPendingNodePaymentCheck ?? {
            if let payments = nodeService.node?.listPayments(), payments.contains(where: {
                if case .pending = $0.status { return true }
                return false
            }) {
                return true
            }
            return false
        }
    }

    func hasPendingOutgoingPayments() -> Bool {
        if hasPendingNodePaymentCheck() {
            return true
        }
        return (try? databaseService.paymentRepo.hasPendingOutgoingPayment()) ?? true
    }
}

/// Adapts SQLite database queries to SpliceStatusProviding.
final class DatabaseSpliceStatusAdapter: SpliceStatusProviding {
    private let databaseService: DatabaseService

    init(databaseService: DatabaseService) {
        self.databaseService = databaseService
    }

    /// Also while the LSP's own splice is negotiated and not locked: the channel is changing under
    /// the books.
    func hasPendingSplice() -> Bool {
        let repo = databaseService.spliceRepo
        return (try? repo.hasPendingSplice() || repo.hasPendingLspSplice()) ?? true
    }
}

/// Adapts SQLite database queries to StabilitySendStatusProviding.
final class DatabaseStabilitySendStatusAdapter: StabilitySendStatusProviding {
    private let databaseService: DatabaseService

    init(databaseService: DatabaseService) {
        self.databaseService = databaseService
    }

    func hasPendingStabilitySend() -> Bool {
        databaseService.stabilityRepo.loadPendingSend() != nil
    }
}
