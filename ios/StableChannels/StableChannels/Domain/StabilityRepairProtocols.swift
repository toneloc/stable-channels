import Foundation

/// Defines a port for querying pending outgoing payment status.
protocol PaymentStatusProviding {
    func hasPendingOutgoingPayments() -> Bool
}

/// Defines a port for querying pending splice status.
protocol SpliceStatusProviding {
    func hasPendingSplice() -> Bool
}

/// Defines a port for querying pending stability settlement status.
protocol StabilitySendStatusProviding {
    func hasPendingStabilitySend() -> Bool
}
