import LDKNode
@testable import StableChannels
import XCTest

final class SendDomainPolicyTests: XCTestCase {
    func testInvoicePayability_expiredWinsOverAnyEarlierPayment() {
        XCTAssertEqual(InvoicePayability.evaluate(isExpired: true, existingOutboundPayment: nil), .expired)
        XCTAssertEqual(InvoicePayability.evaluate(isExpired: true, existingOutboundPayment: .failed), .expired)
    }

    func testInvoicePayability_earlierPaymentDecides() {
        XCTAssertEqual(InvoicePayability.evaluate(isExpired: false, existingOutboundPayment: .succeeded), .alreadyPaid)
        XCTAssertEqual(
            InvoicePayability.evaluate(isExpired: false, existingOutboundPayment: .pending),
            .paymentInProgress
        )
        XCTAssertEqual(InvoicePayability.evaluate(isExpired: false, existingOutboundPayment: .failed), .payable)
        XCTAssertEqual(InvoicePayability.evaluate(isExpired: false, existingOutboundPayment: nil), .payable)
    }

    func testInvoicePayability_onlyPayableHasNoMessage() {
        XCTAssertNil(InvoicePayability.payable.userMessage)
        XCTAssertEqual(
            InvoicePayability.expired.userMessage,
            "This invoice has expired. Ask the recipient for a new one."
        )
        XCTAssertEqual(InvoicePayability.alreadyPaid.userMessage, "This invoice has already been paid.")
        XCTAssertEqual(
            InvoicePayability.paymentInProgress.userMessage,
            "A payment for this invoice is already in progress."
        )
    }

    func testSendSuccessStatus_pendingBeatsTxidAndTxidMeansUnconfirmed() {
        XCTAssertEqual(SendSuccessStatus.of(isPendingSettlement: true, txid: "txid"), .pending)
        XCTAssertEqual(SendSuccessStatus.of(isPendingSettlement: true, txid: nil), .pending)
        XCTAssertEqual(SendSuccessStatus.of(isPendingSettlement: false, txid: "txid"), .awaitingConfirmation)
        XCTAssertEqual(SendSuccessStatus.of(isPendingSettlement: false, txid: nil), .completed)
    }
}
