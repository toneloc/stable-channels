import XCTest
@testable import StableChannels

final class PaymentRowAmountTests: XCTestCase {
    private func payment(direction: String, amountUSD: Double?, btcPrice: Double? = nil) -> PaymentRecord {
        PaymentRecord(
            id: 1,
            paymentId: "p",
            paymentType: "lightning",
            direction: direction,
            amountMsat: 100_000_000,
            amountUSD: amountUSD,
            btcPrice: btcPrice,
            counterparty: nil,
            status: "completed",
            createdAt: 0,
            feeMsat: 0,
            txid: nil,
            address: nil,
            confirmations: 0,
            txBlockHeight: nil
        )
    }

    func testReceivedIsPlusAndSentIsMinus() {
        XCTAssertEqual(payment(direction: "received", amountUSD: 12.5).signedAmountText(fallbackPrice: 0), "+$12.50")
        XCTAssertEqual(payment(direction: "sent", amountUSD: 12.5).signedAmountText(fallbackPrice: 0), "-$12.50")
    }

    func testFallsBackToBtcWithSignWhenNoUsd() {
        let text = payment(direction: "sent", amountUSD: nil).signedAmountText(fallbackPrice: 0)
        XCTAssertTrue(text.hasPrefix("-"))
        XCTAssertTrue(text.hasSuffix("BTC"))
    }

    func testRecentActivityRowCountFollowsAvailableSpace() {
        XCTAssertEqual(RecentActivityView.rowCount(spaceBelow: 40), 1)
        XCTAssertEqual(RecentActivityView.rowCount(spaceBelow: -100), 1)
        XCTAssertEqual(RecentActivityView.rowCount(spaceBelow: 160), 2)
        XCTAssertEqual(RecentActivityView.rowCount(spaceBelow: 220), 3)
        XCTAssertEqual(RecentActivityView.rowCount(spaceBelow: 400), 4)
    }
}
