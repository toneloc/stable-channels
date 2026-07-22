import Foundation
import SwiftUI

/// Coordinates payment detail presentation from any view (home bubble, history rows).
/// Owned by MainTabView so the sheet can be presented above the tab hierarchy.
@Observable
final class PaymentDetailCoordinator {
    var paymentId: Int64?

    func open(_ payment: PaymentRecord) {
        paymentId = payment.id
    }
}
