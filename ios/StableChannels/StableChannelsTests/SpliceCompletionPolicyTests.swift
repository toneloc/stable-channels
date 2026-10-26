@testable import StableChannels
import XCTest

/// Ports Android SpliceOutBaselineAdvanceDecisionTest (#316 review) plus the self-send gate, and
/// the stale splice lock release rule (Android c577ccb1 / 5fb6c459).
final class SpliceCompletionPolicyTests: XCTestCase {
    private func row(
        type: String = "splice_out",
        direction: String = "sent",
        amountSats: UInt64 = 50000
    ) -> PaymentRecord {
        PaymentRecord(
            id: 1,
            paymentId: nil,
            paymentType: type,
            direction: direction,
            amountMsat: amountSats * 1000,
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: "completed",
            createdAt: 0,
            feeMsat: 0,
            txid: "tx",
            address: "bc1qourownaddress",
            confirmations: 1,
            txBlockHeight: nil
        )
    }

    private func baseline(
        prev: UInt64,
        current: UInt64,
        row: PaymentRecord? = nil,
        own: Bool = true
    ) -> UInt64? {
        SpliceCompletionPolicy.advancedOnchainBaseline(
            prevOnchainSats: prev,
            currentOnchainSats: current,
            completedRow: row ?? self.row(),
            destinationIsOwnAddress: own
        )
    }

    // MARK: - Baseline advance

    func testExactSelfSendSettlementCatchesBaselineUpToCurrent() {
        XCTAssertEqual(baseline(prev: 0, current: 50000), 50000)
    }

    func testFeeDriftNeverPushesBaselineAboveCurrentBalance() {
        XCTAssertEqual(baseline(prev: 0, current: 49950), 49950)
    }

    func testConcurrentUnrelatedDepositRemainsVisibleAboveTheNewBaseline() {
        // A separate 10,000-sat deposit landed alongside a 50,000-sat self-send.
        XCTAssertEqual(baseline(prev: 0, current: 60000), 50000)
    }

    func testBaselineAdvancesFromANonZeroStart() {
        XCTAssertEqual(baseline(prev: 20000, current: 80000), 70000)
    }

    func testExternalDestinationNeverMovesTheBaseline() {
        XCTAssertNil(baseline(prev: 0, current: 60000, own: false))
    }

    func testSpliceInNeverMovesTheBaseline() {
        XCTAssertNil(baseline(prev: 0, current: 60000, row: row(type: "splice_in", direction: "received")))
    }

    func testReceivedDirectionNeverMovesTheBaseline() {
        XCTAssertNil(baseline(prev: 0, current: 60000, row: row(direction: "received")))
    }

    func testZeroAmountKeepsTheBaselineWithinCurrent() {
        XCTAssertEqual(baseline(prev: 10000, current: 10000, row: row(amountSats: 0)), 10000)
    }

    func testOverflowClampsToCurrent() {
        XCTAssertEqual(baseline(prev: .max, current: 60000), 60000)
    }

    // MARK: - Stale lock release

    private func release(
        isSweeping: Bool = true,
        hasTxid: Bool = false,
        monitoring: Bool = false,
        pendingRow: Bool? = false
    ) -> Bool {
        SpliceCompletionPolicy.shouldReleaseStaleLock(
            isSweeping: isSweeping,
            hasSpliceTxid: hasTxid,
            isMonitoringConfirmation: monitoring,
            hasPendingSpliceRow: pendingRow
        )
    }

    func testReleasesALockWithNoTxidMonitorOrPendingRow() {
        XCTAssertTrue(release())
    }

    func testNoLockNothingToRelease() {
        XCTAssertFalse(release(isSweeping: false))
    }

    func testNegotiatedTxidKeepsTheLock() {
        XCTAssertFalse(release(hasTxid: true))
    }

    func testRunningMonitorKeepsTheLock() {
        XCTAssertFalse(release(monitoring: true))
    }

    func testPendingRowKeepsTheLock() {
        XCTAssertFalse(release(pendingRow: true))
    }

    func testFailedCheckReadsAsStillPending() {
        XCTAssertFalse(release(pendingRow: nil))
    }
}

final class SpliceReadyStatusTests: XCTestCase {
    func testReadyStatusNamesAMoveOnlyWhileItsLockAndTxidAreHeld() {
        XCTAssertEqual(
            SpliceCompletionPolicy.channelReadyStatus(isSweeping: true, hasSpliceTxid: true),
            "Move pending confirmation"
        )
        XCTAssertEqual(
            SpliceCompletionPolicy.channelReadyStatus(isSweeping: true, hasSpliceTxid: false),
            "Channel is ready"
        )
        XCTAssertEqual(
            SpliceCompletionPolicy.channelReadyStatus(isSweeping: false, hasSpliceTxid: true),
            "Channel is ready"
        )
    }
}
