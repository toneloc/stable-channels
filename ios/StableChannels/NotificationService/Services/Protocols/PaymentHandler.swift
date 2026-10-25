import Foundation
import UserNotifications
import LDKNode

/// Payment handler protocol - each direction has its own handler
protocol PaymentHandler {
    var direction: PaymentDirection { get }
    func handle(
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher,
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool?) -> Void
    )
}

extension PaymentHandler {
    /// Commits an outbound payment outcome before its event is acked, so a result consumed
    /// here is never lost to history. False means leave the event for the foreground.
    func recordOutgoingOutcome(_ event: Event, node: LDKNode.Node, db: PaymentDatabase) -> Bool {
        switch event {
        case .paymentSuccessful(let paymentId, _, _, let feePaidMsat, _):
            guard let paymentId else { return true }
            return db.recordOutgoingPaymentSuccess(paymentId: "\(paymentId)", feeMsat: feePaidMsat)
        case .paymentFailed(let paymentId, _, let reason):
            guard let paymentId else { return true }
            return db.recordOutgoingPaymentFailure(
                paymentId: "\(paymentId)",
                reasonCode: LightningPaymentRecovery.reasonCode(caseName: reason.map { "\($0)" }),
                lookupAmountMsat: {
                    guard let payment = node.payment(paymentId: paymentId),
                          payment.direction == .outbound,
                          case .spontaneous = payment.kind else { return nil }
                    return payment.amountMsat
                }
            )
        default:
            return true
        }
    }
}

/// Factory to create the appropriate handler for a direction
enum PaymentHandlerFactory {
    /// `syncGate` carries the user_to_lsp chain-freshness wait context (see #243);
    /// the other directions ignore it.
    static func handler(for direction: PaymentDirection, syncGate: StabilitySyncGate? = nil) -> PaymentHandler {
        switch direction {
        case .lspToUser: return LSPToUserHandler()
        case .incomingPayment: return IncomingPaymentHandler()
        case .userToLsp: return UserToLSPHandler(syncGate: syncGate)
        }
    }
}
