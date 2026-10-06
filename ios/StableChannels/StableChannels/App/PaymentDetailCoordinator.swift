import Foundation
import SwiftUI

/// Coordinates payment detail presentation from any view (home bubble, history rows).
/// Owned by MainTabView so the sheet can be presented above the tab hierarchy.
@Observable
final class PaymentDetailCoordinator {
    var paymentId: Int64?
    /// Set by Home's "View all"; MainTabView switches to History and HistoryView shows Payments.
    var showPaymentsRequested = false

    func open(_ payment: PaymentRecord) {
        paymentId = payment.id
    }
}
