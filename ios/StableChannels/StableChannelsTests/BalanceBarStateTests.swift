import XCTest
@testable import StableChannels

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

        // Settle fraction takes precedence over default allocation fraction
        XCTAssertEqual(
            state.effectiveFraction(allocation: fundedAllocation, settleFraction: 0.58),
            0.58,
            accuracy: 0.001
        )
    }
}
