import XCTest
@testable import StableChannels

final class BalanceBarTradeCalculatorTests: XCTestCase {
    func testEmptyBalanceReturnsInvalidTrade() {
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.5,
            targetFraction: 0.7,
            totalUSD: 0.0,
            stableUSD: 0.0,
            maxSellUSD: 0.0
        )

        XCTAssertFalse(evaluation.isValidTrade)
        XCTAssertNil(evaluation.direction)
        XCTAssertEqual(evaluation.clampedUSD, 0.0)
    }

    func testBuyUnderMinimumOneDollarIsRejected() {
        // totalUSD = 100, drag left from 0.50 to 0.495 (delta 0.005 -> $0.50)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.50,
            targetFraction: 0.495,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 50.0
        )

        XCTAssertEqual(evaluation.direction, .buy)
        XCTAssertEqual(evaluation.requestedUSD, 0.5, accuracy: 0.001)
        XCTAssertFalse(evaluation.isValidTrade)
    }

    func testBuyAtOrAboveOneDollarIsAccepted() {
        // totalUSD = 100, drag left from 0.50 to 0.49 (delta 0.01 -> $1.00)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.50,
            targetFraction: 0.49,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 50.0
        )

        XCTAssertEqual(evaluation.direction, .buy)
        XCTAssertEqual(evaluation.clampedUSD, 1.0, accuracy: 0.001)
        XCTAssertTrue(evaluation.isValidTrade)
    }

    func testBuyBeyondStableBalanceClampsToAvailableStableUSD() {
        // totalUSD = 100, stableUSD = 10.0, drag from 0.10 to 0.0 (requested $10, or user attempts $20 buy)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.10,
            targetFraction: 0.0,
            totalUSD: 100.0,
            stableUSD: 5.0, // Only $5 stable available
            maxSellUSD: 50.0
        )

        XCTAssertEqual(evaluation.direction, .buy)
        XCTAssertEqual(evaluation.requestedUSD, 10.0, accuracy: 0.001)
        XCTAssertEqual(evaluation.clampedUSD, 5.0, accuracy: 0.001)
        XCTAssertTrue(evaluation.isValidTrade)
    }

    func testSellBeyondMaxSellUSDClampsToLimit() {
        // totalUSD = 100, maxSellUSD = 20.0, drag from 0.50 to 0.80 ($30 requested)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.50,
            targetFraction: 0.80,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 20.0
        )

        XCTAssertEqual(evaluation.direction, .sell)
        XCTAssertEqual(evaluation.requestedUSD, 30.0, accuracy: 0.001)
        XCTAssertEqual(evaluation.clampedUSD, 20.0, accuracy: 0.001)
        XCTAssertTrue(evaluation.isValidTrade)
    }

    func testSellExactlyMaxSellUSDIsAccepted() {
        // totalUSD = 100, maxSellUSD = 25.0, drag from 0.50 to 0.75 ($25 requested)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.50,
            targetFraction: 0.75,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 25.0
        )

        XCTAssertEqual(evaluation.direction, .sell)
        XCTAssertEqual(evaluation.clampedUSD, 25.0, accuracy: 0.001)
        XCTAssertTrue(evaluation.isValidTrade)
    }

    func testClampFractionBeyondPhysicalBounds() {
        // Raw fraction < 0
        let under = BalanceBarTradeCalculator.clampFraction(
            initialFraction: 0.5,
            rawFraction: -0.2,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 50.0
        )
        XCTAssertEqual(under.fraction, 0.0, accuracy: 0.001)
        XCTAssertFalse(under.isAtSellLimit)

        // Raw fraction > 1
        let over = BalanceBarTradeCalculator.clampFraction(
            initialFraction: 0.5,
            rawFraction: 1.5,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 50.0
        )
        XCTAssertEqual(over.fraction, 1.0, accuracy: 0.001)
    }

    func testClampFractionEnforcesSellLimitFlag() {
        // totalUSD = 100, maxSellUSD = 20. Max allowed fraction = 0.5 + 0.2 = 0.7
        let atLimit = BalanceBarTradeCalculator.clampFraction(
            initialFraction: 0.5,
            rawFraction: 0.75,
            totalUSD: 100.0,
            stableUSD: 50.0,
            maxSellUSD: 20.0
        )

        XCTAssertEqual(atLimit.fraction, 0.7, accuracy: 0.001)
        XCTAssertTrue(atLimit.isAtSellLimit)
    }

    func testZeroBtcPriceChannelAllocationAndTrade() {
        let allocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 100_000,
            btcPrice: 0.0
        )

        XCTAssertEqual(allocation.totalUSD, 50.0)
        XCTAssertEqual(allocation.nativeUSD, 0.0)
        XCTAssertEqual(allocation.stableFraction, 1.0)
        XCTAssertFalse(allocation.isEmpty)

        // With 100% USD, user cannot sell (already 1.0)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 1.0,
            targetFraction: 0.8,
            totalUSD: allocation.totalUSD,
            stableUSD: allocation.stableUSD,
            maxSellUSD: 0.0
        )

        XCTAssertEqual(evaluation.direction, .buy)
        XCTAssertEqual(evaluation.clampedUSD, 10.0, accuracy: 0.001)
        XCTAssertTrue(evaluation.isValidTrade)
    }

    func testBtcOnlyBalanceCannotBuyStableUSD() {
        // stableUSD = 0, native balance = $100
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.0,
            targetFraction: 0.0,
            totalUSD: 100.0,
            stableUSD: 0.0,
            maxSellUSD: 50.0
        )
        XCTAssertFalse(evaluation.isValidTrade)

        let tryBuy = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: 0.0,
            targetFraction: -0.1,
            totalUSD: 100.0,
            stableUSD: 0.0,
            maxSellUSD: 50.0
        )
        XCTAssertEqual(tryBuy.clampedUSD, 0.0)
        XCTAssertFalse(tryBuy.isValidTrade)
    }

    func testInteractionMathTranslationAndTapDetection() {
        // Translation math
        let target = BalanceBarTradeCalculator.calculateTargetFraction(
            initialFraction: 0.4,
            translationX: 75.0,
            barWidth: 300.0
        )
        // 0.4 + 75/300 = 0.4 + 0.25 = 0.65
        XCTAssertEqual(target, 0.65, accuracy: 0.001)

        // Thumb position alignment
        let thumbX = BalanceBarTradeCalculator.calculateThumbPosition(
            fraction: 0.65,
            barWidth: 300.0
        )
        XCTAssertEqual(thumbX, 195.0, accuracy: 0.001)

        // Tap detection
        XCTAssertTrue(BalanceBarTradeCalculator.isTap(translationX: 3.0, translationY: 2.0, threshold: 5.0))
        XCTAssertFalse(BalanceBarTradeCalculator.isTap(translationX: 6.0, translationY: 0.0, threshold: 5.0))
    }
}
