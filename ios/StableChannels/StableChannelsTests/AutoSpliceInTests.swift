import XCTest
@testable import StableChannels

final class AutoSpliceInTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_700_000_000)

    private func decide(
        canSplice: Bool = true,
        spendableSats: UInt64 = 100_000,
        confirmations: Int? = 3,
        lastAttemptAt: Date? = nil,
        feeRate: UInt64? = 2
    ) async -> AutoSpliceIn.Step {
        await AutoSpliceIn.decide(
            canSplice: canSplice,
            spendableSats: spendableSats,
            newestConfirmations: { confirmations },
            lastAttemptAt: lastAttemptAt,
            now: now,
            feeRateSatVb: { feeRate }
        )
    }

    func testIdleWhenCannotSplice() async {
        let step = await decide(canSplice: false)
        XCTAssertEqual(step, .idle)
    }

    func testIdleWhenNothingSpendable() async {
        let step = await decide(spendableSats: 0)
        XCTAssertEqual(step, .idle)
    }

    func testConfirmingBelowThreeConfirmations() async {
        let zero = await decide(confirmations: 0)
        let two = await decide(confirmations: 2)
        XCTAssertEqual(zero, .confirming)
        XCTAssertEqual(two, .confirming)
    }

    func testSplicesAtThreeConfirmations() async {
        let step = await decide(confirmations: 3)
        XCTAssertEqual(step, .splice)
    }

    func testUnknownConfirmationsDoNotBlock() async {
        let step = await decide(confirmations: nil)
        XCTAssertEqual(step, .splice)
    }

    func testRetryLaterWithinTenMinutesOfLastAttempt() async {
        let step = await decide(lastAttemptAt: now.addingTimeInterval(-(AutoSpliceIn.retryAfter - 1)))
        XCTAssertEqual(step, .retryLater)
    }

    func testSplicesOnceTenMinutesHavePassed() async {
        let step = await decide(lastAttemptAt: now.addingTimeInterval(-AutoSpliceIn.retryAfter))
        XCTAssertEqual(step, .splice)
    }

    func testRetryLaterWhenFeeRateUnavailable() async {
        let step = await decide(feeRate: nil)
        XCTAssertEqual(step, .retryLater)
    }

    func testAbsoluteFeeCapEdge() async {
        // 29 sat/vB * 170 vB = 4,930 sats is allowed; 30 sat/vB = 5,100 sats is not.
        let atCap = await decide(spendableSats: 1_000_000, feeRate: 29)
        let overCap = await decide(spendableSats: 1_000_000, feeRate: 30)
        XCTAssertEqual(atCap, .splice)
        XCTAssertEqual(overCap, .feeTooHigh)
    }

    func testPercentFeeCapEdge() async {
        // 10 sat/vB * 170 vB = 1,700 sats, which is exactly 50% of 3,400 sats.
        let atCap = await decide(spendableSats: 3_400, feeRate: 10)
        let overCap = await decide(spendableSats: 3_399, feeRate: 10)
        XCTAssertEqual(atCap, .splice)
        XCTAssertEqual(overCap, .feeTooHigh)
    }

    func testOverflowingFeeRateIsTooHigh() async {
        let step = await decide(feeRate: .max)
        XCTAssertEqual(step, .feeTooHigh)
    }

    func testClosuresNotCalledWhenAnEarlierStepReturns() async {
        var confirmationCalls = 0
        var feeCalls = 0

        let idle = await AutoSpliceIn.decide(
            canSplice: false,
            spendableSats: 100_000,
            newestConfirmations: { confirmationCalls += 1; return 3 },
            lastAttemptAt: nil,
            now: now,
            feeRateSatVb: { feeCalls += 1; return 2 }
        )
        XCTAssertEqual(idle, .idle)
        XCTAssertEqual(confirmationCalls, 0)
        XCTAssertEqual(feeCalls, 0)

        let confirming = await AutoSpliceIn.decide(
            canSplice: true,
            spendableSats: 100_000,
            newestConfirmations: { confirmationCalls += 1; return 2 },
            lastAttemptAt: nil,
            now: now,
            feeRateSatVb: { feeCalls += 1; return 2 }
        )
        XCTAssertEqual(confirming, .confirming)
        XCTAssertEqual(confirmationCalls, 1)
        XCTAssertEqual(feeCalls, 0)

        let retryLater = await AutoSpliceIn.decide(
            canSplice: true,
            spendableSats: 100_000,
            newestConfirmations: { 3 },
            lastAttemptAt: now,
            now: now,
            feeRateSatVb: { feeCalls += 1; return 2 }
        )
        XCTAssertEqual(retryLater, .retryLater)
        XCTAssertEqual(feeCalls, 0)
    }

    func testNewestConfirmationsNilWithoutConfirmedHeights() {
        XCTAssertNil(AutoSpliceIn.newestConfirmations(confirmedHeights: [], tipHeight: 900_000))
    }

    func testNewestConfirmationsUsesHighestHeight() {
        let confirmations = AutoSpliceIn.newestConfirmations(
            confirmedHeights: [899_990, 899_998, 899_995],
            tipHeight: 900_000
        )
        XCTAssertEqual(confirmations, 3)
    }

    func testNewestConfirmationsCountsTipBlockAsOne() {
        XCTAssertEqual(AutoSpliceIn.newestConfirmations(confirmedHeights: [900_000], tipHeight: 900_000), 1)
    }

    func testNewestConfirmationsNeverNegative() {
        XCTAssertEqual(AutoSpliceIn.newestConfirmations(confirmedHeights: [900_005], tipHeight: 900_000), 0)
    }
}
