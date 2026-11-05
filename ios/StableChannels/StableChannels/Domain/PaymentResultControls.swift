import Foundation

/// Which dismiss controls a payment result screen offers. Done means "this is final", so it
/// only appears once the outcome is definitive; a pending result keeps a Close instead.
enum PaymentResultControls {
    /// A trade is final only once a signed acceptance or rejection has been recorded.
    static func showsTradeDone(outcome: TradeOutcome?) -> Bool {
        outcome != nil
    }

    /// A Lightning send is pending until its payment settles or fails. Sends without a payment id
    /// (on-chain, splice-out) are never pending Lightning payments.
    static func isLightningPending(paymentId: String?, isPendingSettlement: Bool) -> Bool {
        paymentId != nil && isPendingSettlement
    }
}
