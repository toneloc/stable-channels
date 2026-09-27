import XCTest
@testable import StableChannels

final class ChannelAllocationTests: XCTestCase {
    func testZeroPriceReturnsZero() {
        let allocation = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 200_000,
            btcPrice: 0.0
        )

        XCTAssertEqual(allocation.stableSats, 0)
        XCTAssertEqual(allocation.nativeSats, 200_000)
        XCTAssertEqual(allocation.nativeUSD, 0.0)
        XCTAssertEqual(allocation.totalUSD, 100.0)
        XCTAssertEqual(allocation.stableFraction, 1.0)
    }

    func testStandardAllocation() {
        // At $100,000 per BTC:
        // 1 BTC = 100,000,000 sats = $100,000.
        // $1 = 1,000 sats.
        // $50 stable = 50,000 sats.
        let allocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 150_000,
            btcPrice: 100_000.0
        )

        XCTAssertEqual(allocation.stableSats, 50_000)
        XCTAssertEqual(allocation.nativeSats, 100_000)
        XCTAssertEqual(allocation.nativeUSD, 100.0)
        XCTAssertEqual(allocation.totalUSD, 150.0)
        XCTAssertEqual(allocation.stableFraction, 50.0 / 150.0, accuracy: 0.0001)
    }

    func testDeficitStableDoesNotUnderflowNativeSats() {
        // When channel balance has fewer sats than the stable position theoretically requires
        let allocation = ChannelAllocation(
            stableUSD: 200.0,
            lightningBalanceSats: 50_000,
            btcPrice: 100_000.0
        )

        // $200 would require 200,000 sats, but channel only has 50,000 sats
        XCTAssertEqual(allocation.stableSats, 200_000)
        XCTAssertEqual(allocation.nativeSats, 0)
        XCTAssertEqual(allocation.nativeUSD, 0.0)
        XCTAssertEqual(allocation.totalUSD, 200.0)
        XCTAssertEqual(allocation.stableFraction, 1.0)
    }

    func testZeroBalances() {
        let allocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 100_000.0
        )

        XCTAssertEqual(allocation.stableSats, 0)
        XCTAssertEqual(allocation.nativeSats, 0)
        XCTAssertEqual(allocation.nativeUSD, 0.0)
        XCTAssertEqual(allocation.totalUSD, 0.0)
        XCTAssertEqual(allocation.stableFraction, 0.0)
    }

    func testBackingSatsOverrideTakesPrecedence() {
        let allocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 150_000,
            btcPrice: 100_000.0,
            backingSatsOverride: 52_000
        )

        // Mathematical calculation would be 50,000 sats, but backingSatsOverride specifies 52,000 sats
        XCTAssertEqual(allocation.stableSats, 52_000)
        XCTAssertEqual(allocation.nativeSats, 98_000)
    }

    func testBackingSatsOverridePreservedWhenNoStableUSD() {
        let allocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 150_000,
            btcPrice: 100_000.0,
            backingSatsOverride: 52_000
        )

        // When backingSatsOverride is provided in a $0 target state (e.g. unsettled LSP surplus),
        // the override is authoritative so native sats is receiverSats - backingSats.
        XCTAssertEqual(allocation.stableSats, 52_000)
        XCTAssertEqual(allocation.nativeSats, 98_000)
    }

    func testBackingSatsOverrideClampedToLightningBalance() {
        let allocation = ChannelAllocation(
            stableUSD: 50.0,
            lightningBalanceSats: 40_000,
            btcPrice: 100_000.0,
            backingSatsOverride: 50_000
        )

        XCTAssertEqual(allocation.stableSats, 40_000)
        XCTAssertEqual(allocation.nativeSats, 0)
    }

    func testSplitBrainPreventionWithBackingSatsOverride() {
        // $100 target, 100,000 channel sats, 100,000 backing sats, $110,000/BTC.
        let withOverride = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: 110_000.0,
            backingSatsOverride: 100_000
        )
        // Backing override locks in 100,000 sats -> 0 native sats
        XCTAssertEqual(withOverride.stableSats, 100_000)
        XCTAssertEqual(withOverride.nativeSats, 0)
        XCTAssertEqual(withOverride.nativeUSD, 0.0)

        // Without override, mark-to-market derives ~90,909 stable and 9,091 native sats
        let withoutOverride = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: 110_000.0
        )
        XCTAssertEqual(withoutOverride.stableSats, 90_909)
        XCTAssertEqual(withoutOverride.nativeSats, 9_091)
    }

    func testPathologicalPricesDoNotTrap() {
        // Extreme positive price close to zero
        let tinyPrice = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: 1e-9
        )
        // Must clamp or overflow safely without trapping/crashing
        XCTAssertGreaterThanOrEqual(tinyPrice.stableSats, 0)

        // Negative price
        let negativePrice = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: -50_000.0
        )
        XCTAssertEqual(negativePrice.stableSats, 0)

        // NaN and Infinity
        let nanPrice = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: Double.nan
        )
        XCTAssertEqual(nanPrice.stableSats, 0)

        let infPrice = ChannelAllocation(
            stableUSD: 100.0,
            lightningBalanceSats: 100_000,
            btcPrice: Double.infinity
        )
        XCTAssertEqual(infPrice.stableSats, 0)
    }

    func testStableFractionBounds() {
        let overfunded = ChannelAllocation(
            stableUSD: 500.0,
            lightningBalanceSats: 10_000,
            btcPrice: 100_000.0
        )
        XCTAssertLessThanOrEqual(overfunded.stableFraction, 1.0)
        XCTAssertGreaterThanOrEqual(overfunded.stableFraction, 0.0)

        let zeroBalance = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 0,
            btcPrice: 100_000.0
        )
        XCTAssertEqual(zeroBalance.stableFraction, 0.0)
    }
}
