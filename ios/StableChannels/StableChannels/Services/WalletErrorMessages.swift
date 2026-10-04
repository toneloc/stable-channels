import Foundation
import LDKNode

/// User-facing copy mapped from low-level failure reasons and exceptions.
/// Keep technical details and raw exceptions in telemetry/logs, not in UI banners.
enum WalletErrorMessages {
    static func paymentFailure(_ reason: PaymentFailureReason?) -> String {
        guard let reason else {
            return "The payment did not complete. Check its status in History before trying again."
        }
        switch reason {
        case .retriesExhausted:
            return "The payment could not complete after several attempts. Try a smaller amount or try again later."
        case .routeNotFound:
            return "No available Lightning route could carry this payment. Try a smaller amount or try again later."
        case .recipientRejected:
            return "The recipient rejected the payment. Ask them for a new invoice."
        case .userAbandoned:
            return "The payment was cancelled."
        case .paymentExpired:
            return "The payment expired before it completed. Ask the recipient for a new invoice."
        case .unknownRequiredFeatures:
            return "This wallet does not support a feature required by the invoice. Update the app or ask for a different invoice."
        case .invoiceRequestExpired:
            return "The recipient did not return an invoice in time. Check your connection and try again later."
        case .invoiceRequestRejected:
            return "The recipient rejected the invoice request. Check the offer with them before trying again."
        case .blindedPathCreationFailed:
            return "The wallet could not establish a reply path for this payment. Check your connection and try again later."
        case .unexpectedError:
            return "An unexpected error stopped the payment. Try again later. If it continues, share your logs with support."
        @unknown default:
            return "The payment did not complete. Check its status in History before trying again."
        }
    }

    static func operation(_ error: Error, fallback: String) -> String {
        if let nodeError = error as? NodeError {
            switch nodeError {
            case .AlreadyRunning:
                return "The wallet is already running."
            case .NotRunning:
                return "The wallet is still starting. Wait for it to connect and try again."
            case .ConnectionFailed:
                return "The Lightning provider is unavailable. Check your connection and try again later."
            case .InvoiceCreationFailed, .InvoiceRequestCreationFailed, .OfferCreationFailed:
                return "The wallet could not create an invoice or offer. Check your connection and try again."
            case .PaymentSendingFailed:
                return "The wallet could not send the payment. Check your connection and available balance, then try again."
            case .InvalidCustomTlvs:
                return "The wallet could not prepare the provider message. Update the app and try again."
            case .ChannelCreationFailed, .ChannelSplicingFailed:
                return "The channel operation could not start. Check your connection and available balance, then try again."
            case .ChannelClosingFailed:
                return "The channel could not close. Check your connection to the provider and try again later."
            case .PersistenceFailed:
                return "The wallet could not save the result. Check History before trying again. If it continues, share your logs with support."
            case .FeerateEstimationUpdateFailed, .FeerateEstimationUpdateTimeout,
                 .TxSyncFailed, .TxSyncTimeout,
                 .GossipUpdateFailed, .GossipUpdateTimeout:
                return "The wallet could not update network information. Check your connection and wait for it to sync."
            default:
                return fallback
            }
        }
        let message = error.localizedDescription
        return message.isEmpty ? fallback : message
    }
}
