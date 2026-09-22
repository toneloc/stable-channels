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

    func testBackingSatsOverrideZeroWhenNoStableUSD() {
        let allocation = ChannelAllocation(
            stableUSD: 0.0,
            lightningBalanceSats: 150_000,
            btcPrice: 100_000.0,
            backingSatsOverride: 52_000
        )

        // With zero stable position, stable sats must be zero regardless of override
        XCTAssertEqual(allocation.stableSats, 0)
        XCTAssertEqual(allocation.nativeSats, 150_000)
    }
}
