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

// MARK: - LNURL invoice quote

/// How an LNURL-pay invoice's description relates to the payee metadata.
enum LNURLDescriptionBinding: Equatable, Sendable {
    case matchesMetadata
    case mismatchedHash(invoiceHash: String)
    case direct
}

/// A validated LNURL-pay invoice fetched for the confirm step. Once quoted it fixes the amount the
/// confirm step shows, checks and pays; the slide pays exactly this invoice, and only while it is unexpired.
struct LNURLInvoiceQuote: Equatable, Sendable {
    let invoice: String
    let amountMsat: UInt64
    /// The comment sent to the payee, normalized by `normalizedComment`.
    let comment: String?
    let binding: LNURLDescriptionBinding
    let expiresAt: Date

    /// Trimmed comment, or nil when blank: what is sent to the payee.
    static func normalizedComment(_ comment: String) -> String? {
        let trimmed = comment.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The invoice is not bound to the payee metadata. Custom-domain addresses that forward to a provider
    /// cause this legitimately, so it is audit-logged rather than shown.
    var isDescriptionUnbound: Bool {
        binding != .matchesMetadata
    }

    /// Treated as expired this long before its real expiry: authentication can take tens of seconds before dispatch.
    static let expiryMarginSecs: TimeInterval = 30

    func isExpired(at now: Date) -> Bool {
        now.addingTimeInterval(Self.expiryMarginSecs) >= expiresAt
    }
}
