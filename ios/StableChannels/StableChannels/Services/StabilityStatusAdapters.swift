import Foundation

/// Adapts LDK Node and SQLite database checks to PaymentStatusProviding.
final class LDKPaymentStatusAdapter: PaymentStatusProviding {
    private let nodeService: NodeService
    private let databaseService: DatabaseService
    private let hasPendingNodePaymentCheck: () -> Bool

    init(
        nodeService: NodeService,
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

    func hasPendingSplice() -> Bool {
        (try? databaseService.spliceRepo.hasPendingSplice()) ?? true
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
