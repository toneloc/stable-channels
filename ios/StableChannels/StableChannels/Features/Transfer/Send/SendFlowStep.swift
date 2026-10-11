import Foundation

/// Active step in the sequential Send workflow.
enum SendFlowStep: Equatable, Sendable {
    case recipient
    case amount
    case confirm
    case success
}

/// How final a dispatched payment is, as shown on the result screen.
enum SendSuccessStatus: Equatable, Sendable {
    /// Lightning payment settled.
    case completed
    /// On-chain transaction broadcast but not yet confirmed.
    case awaitingConfirmation
    /// Lightning settlement or splice-out still in flight.
    case pending
}
