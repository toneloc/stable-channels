import Foundation

/// A trade's terminal result: a signed acceptance, or a rejection with the message to show.
/// Absence means the outcome is not known yet.
struct TradeOutcome: Equatable {
    let accepted: Bool
    let message: String
}
