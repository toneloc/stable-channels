import XCTest
import LDKNode
@testable import StableChannels

final class WalletErrorMessagesTests: XCTestCase {
    func testPaymentFailureReasons() {
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.retriesExhausted),
            "The payment could not complete after several attempts. Try a smaller amount or try again later."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.routeNotFound),
            "No available Lightning route could carry this payment. Try a smaller amount or try again later."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.recipientRejected),
            "The recipient rejected the payment. Ask them for a new invoice."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.userAbandoned),
            "The payment was cancelled."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.paymentExpired),
            "The payment expired before it completed. Ask the recipient for a new invoice."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.unknownRequiredFeatures),
            "This wallet does not support a feature required by the invoice. Update the app or ask for a different invoice."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.invoiceRequestExpired),
            "The recipient did not return an invoice in time. Check your connection and try again later."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.invoiceRequestRejected),
            "The recipient rejected the invoice request. Check the offer with them before trying again."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.blindedPathCreationFailed),
            "The wallet could not establish a reply path for this payment. Check your connection and try again later."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(.unexpectedError),
            "An unexpected error stopped the payment. Try again later. If it continues, share your logs with support."
        )
        XCTAssertEqual(
            WalletErrorMessages.paymentFailure(nil),
            "The payment did not complete. Check its status in History before trying again."
        )
    }

    func testNodeErrors() {
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.AlreadyRunning(message: "already running"), fallback: "Failed"),
            "The wallet is already running."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.NotRunning(message: "not running"), fallback: "Failed"),
            "The wallet is still starting. Wait for it to connect and try again."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.ConnectionFailed(message: "failed"), fallback: "Failed"),
            "The Lightning provider is unavailable. Check your connection and try again later."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.InvoiceCreationFailed(message: "failed"), fallback: "Failed"),
            "The wallet could not create an invoice or offer. Check your connection and try again."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.PaymentSendingFailed(message: "failed"), fallback: "Failed"),
            "The wallet could not send the payment. Check your connection and available balance, then try again."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.InvalidCustomTlvs(message: "failed"), fallback: "Failed"),
            "The wallet could not prepare the provider message. Update the app and try again."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.ChannelCreationFailed(message: "failed"), fallback: "Failed"),
            "The channel operation could not start. Check your connection and available balance, then try again."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.ChannelClosingFailed(message: "failed"), fallback: "Failed"),
            "The channel could not close. Check your connection to the provider and try again later."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.PersistenceFailed(message: "failed"), fallback: "Failed"),
            "The wallet could not save the result. Check History before trying again. If it continues, share your logs with support."
        )
        XCTAssertEqual(
            WalletErrorMessages.operation(NodeError.TxSyncFailed(message: "failed"), fallback: "Failed"),
            "The wallet could not update network information. Check your connection and wait for it to sync."
        )
    }

    func testGenericErrorFallback() {
        struct CustomError: LocalizedError {
            var errorDescription: String? { "" }
        }
        XCTAssertEqual(
            WalletErrorMessages.operation(CustomError(), fallback: "Custom fallback message"),
            "Custom fallback message"
        )
    }
}
