import Foundation

/// How final a dispatched payment is, as shown on the result screen.
enum SendSuccessStatus: Equatable, Sendable {
    /// Lightning payment settled.
    case completed
    /// On-chain transaction broadcast but not yet confirmed.
    case awaitingConfirmation
    /// Lightning settlement or splice-out still in flight.
    case pending

    /// Unsettled sends stay pending; a broadcast transaction awaits confirmation; anything else settled.
    static func of(isPendingSettlement: Bool, txid: String?) -> SendSuccessStatus {
        if isPendingSettlement { return .pending }
        return txid != nil ? .awaitingConfirmation : .completed
    }
}
