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
        completion: @escaping (UNMutableNotificationContent, Bool) -> Void
    )
}

/// Factory to create the appropriate handler for a direction
enum PaymentHandlerFactory {
    static func handler(for direction: PaymentDirection) -> PaymentHandler {
        switch direction {
        case .lspToUser: return LSPToUserHandler()
        case .incomingPayment: return IncomingPaymentHandler()
        case .userToLsp: return UserToLSPHandler()
        }
    }
}
