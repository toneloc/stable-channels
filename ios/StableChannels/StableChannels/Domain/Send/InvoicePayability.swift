import Foundation
import LDKNode

/// Whether a BOLT11 invoice can still be paid by this wallet.
enum InvoicePayability: Equatable, Sendable {
    case payable
    case expired
    case alreadyPaid
    case paymentInProgress

    /// An expired invoice is never payable; otherwise an earlier outbound payment for the same
    /// hash decides: settled means paid, pending means in flight, failed allows a retry.
    static func evaluate(isExpired: Bool, existingOutboundPayment: PaymentStatus?) -> InvoicePayability {
        if isExpired { return .expired }
        switch existingOutboundPayment {
        case .succeeded: return .alreadyPaid
        case .pending: return .paymentInProgress
        case .failed, .none: return .payable
        }
    }

    /// Why the invoice cannot be paid, or nil when it can.
    var userMessage: String? {
        switch self {
        case .payable: nil
        case .expired: "This invoice has expired. Ask the recipient for a new one."
        case .alreadyPaid: "This invoice has already been paid."
        case .paymentInProgress: "A payment for this invoice is already in progress."
        }
    }
}
