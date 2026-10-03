import XCTest
@testable import StableChannels

private final class SpyBalanceBarHaptics: BalanceBarHaptics {
    var tickCount = 0
    var impactCount = 0
    var warningCount = 0

    func tick() { tickCount += 1 }
    func impact() { impactCount += 1 }
    func warning() { warningCount += 1 }
}

final class BalanceBarStateTests: XCTestCase {
    func testEmptyStateEffectiveFractionDefaultsToCenter() {
        let state = BalanceBarState()
        let emptyAllocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 50000.0
        )

        XCTAssertEqual(state.effectiveFraction(allocation: emptyAllocation, settleFraction: nil), 0.5)
        XCTAssertNil(state.userSelectedFraction)
        XCTAssertFalse(state.showDepositPrompt)
    }

    func testUserSelectedFractionOverridesCanonicalFraction() {
        let state = BalanceBarState()
        let emptyAllocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 50000.0
        )

        state.userSelectedFraction = 0.72
        XCTAssertEqual(state.effectiveFraction(allocation: emptyAllocation, settleFraction: nil), 0.72)

        state.resetSelection()
        XCTAssertNil(state.userSelectedFraction)
        XCTAssertEqual(state.effectiveFraction(allocation: emptyAllocation, settleFraction: nil), 0.5)
    }

    func testNonEmptyEffectiveFractionUsesAllocationFraction() {
        let state = BalanceBarState()
        let fundedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 200_000,
            btcPrice: 100_000.0
        )

        XCTAssertEqual(
            state.effectiveFraction(allocation: fundedAllocation, settleFraction: nil),
            0.25,
            accuracy: 0.001
        )
    }

    func testSettleFractionPrecedenceDuringAwakening() {
        let state = BalanceBarState()
        let fundedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 200_000,
            btcPrice: 100_000.0
        )

        XCTAssertEqual(
            state.effectiveFraction(allocation: fundedAllocation, settleFraction: 0.58),
            0.58,
            accuracy: 0.001
        )
    }

    func testEmptyDragUnderThresholdTriggersTapAction() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let emptyAllocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 50000.0
        )
        var emptyActionCalled = false

        state.handleDragChange(
            touchStartX: 150.0,
            translationX: 3.0,
            barWidth: 300.0,
            currentThumbX: 150.0,
            thumbDiameter: 22.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            onDragStarted: nil
        )

        XCTAssertTrue(state.isPressing)
        XCTAssertEqual(spy.tickCount, 1)

        state.handleDragEnd(
            translationX: 3.0,
            barWidth: 300.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            reduceMotion: false,
            onEmptyInteraction: { emptyActionCalled = true },
            onTradeRequest: nil
        )

        XCTAssertFalse(state.isPressing)
        XCTAssertTrue(emptyActionCalled)
        XCTAssertFalse(state.showDepositPrompt)
        XCTAssertEqual(spy.tickCount, 2)
    }

    func testEmptyDragOverThresholdShowsDepositPrompt() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let emptyAllocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 50000.0
        )
        var emptyActionCalled = false

        // barWidth = 300.0, thumbDiameter = 22.0 -> usableWidth = 278.0
        // translationX = 55.6 -> 55.6 / 278.0 = 0.20 -> 0.50 + 0.20 = 0.70
        state.handleDragChange(
            touchStartX: 150.0,
            translationX: 55.6,
            barWidth: 300.0,
            currentThumbX: 150.0,
            thumbDiameter: 22.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            onDragStarted: nil
        )

        // 0.5 + 55.6/278 = 0.70
        XCTAssertEqual(state.userSelectedFraction ?? 0, 0.70, accuracy: 0.001)

        state.handleDragEnd(
            translationX: 55.6,
            barWidth: 300.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            reduceMotion: false,
            onEmptyInteraction: { emptyActionCalled = true },
            onTradeRequest: nil
        )

        XCTAssertFalse(emptyActionCalled)
        XCTAssertTrue(state.showDepositPrompt)
    }

    func testNonEmptyDragClampsToSellLimitAndTriggersHaptics() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let fundedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 200_000,
            btcPrice: 100_000.0
        )
        // totalUSD = 200, stableFraction = 0.25, maxSellUSD = 25 -> maxSellFraction = 0.125. Limit = 0.375
        let maxSellUSD = 25.0

        state.handleDragChange(
            touchStartX: 75.0,
            translationX: 60.0, // proposed 0.25 + 60/278 = 0.466 (overshoots 0.375 limit)
            barWidth: 300.0,
            currentThumbX: 75.0,
            thumbDiameter: 22.0,
            allocation: fundedAllocation,
            maxSellUSD: maxSellUSD,
            isAwakening: false,
            onDragStarted: nil
        )

        XCTAssertEqual(state.userSelectedFraction ?? 0, 0.375, accuracy: 0.001)
        XCTAssertTrue(state.atSellLimit)
        XCTAssertEqual(spy.warningCount, 1)
    }

    func testTradeRequestTriggeredOnDragEnd() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let fundedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 200_000,
            btcPrice: 100_000.0
        )
        var receivedRequest: TradeRequest?

        // barWidth = 300.0, thumbDiameter = 22.0 -> usableWidth = 278.0
        // translationX = 27.8 -> 27.8 / 278.0 = 0.10 -> 0.25 + 0.10 = 0.35 (fractionMoved 0.10 -> $20.0 sell)
        state.handleDragChange(
            touchStartX: 75.0,
            translationX: 27.8,
            barWidth: 300.0,
            currentThumbX: 75.0,
            thumbDiameter: 22.0,
            allocation: fundedAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            onDragStarted: nil
        )

        state.handleDragEnd(
            translationX: 27.8,
            barWidth: 300.0,
            allocation: fundedAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            reduceMotion: false,
            onEmptyInteraction: nil,
            onTradeRequest: { request in receivedRequest = request }
        )

        XCTAssertNotNil(receivedRequest)
        XCTAssertEqual(receivedRequest?.direction, .sell)
        XCTAssertEqual(receivedRequest?.amountUSD ?? 0, 20.0, accuracy: 0.001)
        XCTAssertEqual(spy.impactCount, 1)
    }

    func testNullOnTradeRequestSnapsBackWithoutLeavingThumbStranded() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let fundedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 200_000,
            btcPrice: 100_000.0
        )

        state.handleDragChange(
            touchStartX: 75.0,
            translationX: 30.0,
            barWidth: 300.0,
            currentThumbX: 75.0,
            thumbDiameter: 22.0,
            allocation: fundedAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            onDragStarted: nil
        )

        XCTAssertNotNil(state.userSelectedFraction)

        state.handleDragEnd(
            translationX: 30.0,
            barWidth: 300.0,
            allocation: fundedAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            reduceMotion: false,
            onEmptyInteraction: nil,
            onTradeRequest: nil // Trade request handler is nil
        )

        // When trade request handler is nil, it animates userSelectedFraction to nil and does NOT fire trade haptic
        XCTAssertNil(state.userSelectedFraction)
        XCTAssertEqual(spy.impactCount, 0)
    }

    func testCumulativeDragTravelDistinguishesTapFromBackAndForthDrag() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let emptyAllocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 50000.0
        )
        var emptyActionCalled = false

        // Start drag at center (150px)
        state.handleDragChange(
            touchStartX: 150.0,
            translationX: 40.0, // Drag right 40px
            barWidth: 300.0,
            currentThumbX: 150.0,
            thumbDiameter: 22.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            onDragStarted: nil
        )

        // Drag back to start (translationX = 0, but cumulative travel = 40 + 40 = 80px)
        state.handleDragChange(
            touchStartX: 150.0,
            translationX: 0.0,
            barWidth: 300.0,
            currentThumbX: 150.0,
            thumbDiameter: 22.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            onDragStarted: nil
        )

        XCTAssertEqual(state.cumulativeDragDistance, 80.0, accuracy: 0.001)

        state.handleDragEnd(
            translationX: 0.0,
            barWidth: 300.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            reduceMotion: false,
            onEmptyInteraction: { emptyActionCalled = true },
            onTradeRequest: nil
        )

        // Must NOT be treated as a tap because cumulative distance exceeds threshold!
        XCTAssertFalse(emptyActionCalled)
        XCTAssertTrue(state.showDepositPrompt)
    }

    func testSellLimitHapticFiresOnlyOnEdgeTransition() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let fundedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 200_000,
            btcPrice: 100_000.0
        )
        let maxSellUSD = 25.0 // limit at 0.375 (37.5px past 75px base)

        // Overshoot limit
        state.handleDragChange(
            touchStartX: 75.0,
            translationX: 60.0,
            barWidth: 300.0,
            currentThumbX: 75.0,
            thumbDiameter: 22.0,
            allocation: fundedAllocation,
            maxSellUSD: maxSellUSD,
            isAwakening: false,
            onDragStarted: nil
        )
        XCTAssertEqual(spy.warningCount, 1)

        // Drag further beyond limit -> warningCount must remain 1
        state.handleDragChange(
            touchStartX: 75.0,
            translationX: 70.0,
            barWidth: 300.0,
            currentThumbX: 75.0,
            thumbDiameter: 22.0,
            allocation: fundedAllocation,
            maxSellUSD: maxSellUSD,
            isAwakening: false,
            onDragStarted: nil
        )
        XCTAssertEqual(spy.warningCount, 1)
    }

    func testPriceTickMidGesturePreservesDeliveredTradeAmountAndDirection() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let initialAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 150_000,
            btcPrice: 100_000.0 // totalUSD = $200.0, stableFraction = 0.25
        )
        var receivedRequest: TradeRequest?

        // Drag right 27.8pt (27.8 / 278 = +0.10 fraction delta -> 0.35)
        state.handleDragChange(
            touchStartX: 75.0,
            translationX: 27.8,
            barWidth: 300.0,
            currentThumbX: 75.0,
            thumbDiameter: 22.0,
            allocation: initialAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            onDragStarted: nil
        )

        // Mid-gesture price tick arrives: BTC drops from $100k to $80k
        // stableSats = 62,500, nativeSats = 87,500 -> nativeUSD = $70.0
        // totalUSD = $120.0, stableFraction = 50 / 120 = 0.4167
        let tickedAllocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 150_000,
            btcPrice: 80_000.0
        )

        // Drag ends after the price tick
        state.handleDragEnd(
            translationX: 27.8,
            barWidth: 300.0,
            allocation: tickedAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            reduceMotion: false,
            onEmptyInteraction: nil,
            onTradeRequest: { request in receivedRequest = request }
        )

        // Direction must remain SELL (not flip to BUY due to the base shift)
        XCTAssertNotNil(receivedRequest)
        XCTAssertEqual(receivedRequest?.direction, .sell)
        // Amount is delta fraction (0.10) * new totalUSD ($120.0) = $12.00
        XCTAssertEqual(receivedRequest?.amountUSD ?? 0, 12.0, accuracy: 0.01)
    }

    func testGestureStartedEmptySnapsBackEvenIfFundsArriveMidGestureUnderReduceMotion() {
        let spy = SpyBalanceBarHaptics()
        let state = BalanceBarState(haptics: spy)
        let emptyAllocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 100_000.0
        )
        var receivedRequest: TradeRequest?
        var emptyActionCalled = false

        // Begin drag on empty playground
        state.handleDragChange(
            touchStartX: 150.0,
            translationX: 55.6,
            barWidth: 300.0,
            currentThumbX: 150.0,
            thumbDiameter: 22.0,
            allocation: emptyAllocation,
            maxSellUSD: 0.0,
            isAwakening: false,
            onDragStarted: nil
        )

        // Funds land mid-gesture
        let fundedAllocation = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: 100_000.0
        )

        // Under Reduce Motion, awakening animation is skipped (isAwakening stays false)
        state.handleDragEnd(
            translationX: 55.6,
            barWidth: 300.0,
            allocation: fundedAllocation,
            maxSellUSD: 50.0,
            isAwakening: false,
            reduceMotion: true,
            onEmptyInteraction: { emptyActionCalled = true },
            onTradeRequest: { request in receivedRequest = request }
        )

        // Must refuse to deliver funded trade because gesture started empty
        XCTAssertNil(receivedRequest)
        XCTAssertFalse(emptyActionCalled)
        XCTAssertNil(state.userSelectedFraction)
    }

    func testLiveThumbDiameterConstantMatchesAndroid() {
        XCTAssertEqual(BalanceBarView.defaultThumbDiameter, 22.0)
    }
}
