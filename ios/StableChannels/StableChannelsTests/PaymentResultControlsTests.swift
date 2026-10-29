@testable import StableChannels
import XCTest

final class PaymentResultControlsTests: XCTestCase {
    func testUnresolvedTradeHidesDone() {
        XCTAssertFalse(PaymentResultControls.showsTradeDone(outcome: nil))
    }

    func testDefinitiveTradeResultsShowDone() {
        for accepted in [true, false] {
            let outcome = TradeOutcome(accepted: accepted, message: "")
            XCTAssertTrue(PaymentResultControls.showsTradeDone(outcome: outcome), "accepted: \(accepted)")
        }
    }

    func testLightningPaymentRemainsPendingUntilItSettles() {
        XCTAssertTrue(PaymentResultControls.isLightningPending(paymentId: "payment", isPendingSettlement: true))
    }

    func testSettledLightningPaymentIsNotPending() {
        XCTAssertFalse(PaymentResultControls.isLightningPending(paymentId: "payment", isPendingSettlement: false))
    }

    func testNonLightningResultsAreNotTreatedAsPendingLightningPayments() {
        // A splice-out stays pending with no payment id; it must not lose its Done.
        XCTAssertFalse(PaymentResultControls.isLightningPending(paymentId: nil, isPendingSettlement: true))
        XCTAssertFalse(PaymentResultControls.isLightningPending(paymentId: nil, isPendingSettlement: false))
    }
}
