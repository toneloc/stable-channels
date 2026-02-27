import XCTest
@testable import StableChannels

final class StabilityServiceTests: XCTestCase {

    // MARK: - Helper

    private func testSC(expectedUSD: Double, price: Double, receiverSats: UInt64) -> StableChannel {
        let backing: UInt64 = price > 0
            ? UInt64(expectedUSD / price * 100_000_000.0)
            : 0
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: expectedUSD)
        sc.backingSats = backing
        sc.latestPrice = price
        sc.stableReceiverBTC = Bitcoin(sats: receiverSats)
        sc.isStableReceiver = true
        return sc
    }

    // MARK: - reconcileOutgoing

    func testOutgoingNoStablePosition() {
        var sc = testSC(expectedUSD: 0.0, price: 100_000.0, receiverSats: 500_000)
        XCTAssertNil(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
    }

    func testOutgoingCoveredByNative() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 800_000)
        XCTAssertNil(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
        XCTAssertEqual(sc.expectedUSD.amount, 500.0)
    }

    func testOutgoingEatsIntoStable() {
        var sc = testSC(expectedUSD: 1000.0, price: 100_000.0, receiverSats: 900_000)
        let deducted = StabilityService.reconcileOutgoing(&sc, price: 100_000.0)
        XCTAssertNotNil(deducted)
        XCTAssertEqual(deducted!, 100.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 900.0, accuracy: 0.01)
        let expectedBacking = UInt64(900.0 / 100_000.0 * 100_000_000.0)
        XCTAssertEqual(sc.backingSats, expectedBacking)
    }

    func testOutgoingPartialStableDeduction() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 300_000)
        let deducted = StabilityService.reconcileOutgoing(&sc, price: 100_000.0)!
        XCTAssertEqual(deducted, 200.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 300.0, accuracy: 0.01)
    }

    func testOutgoingSpendsEntireStable() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 0)
        let deducted = StabilityService.reconcileOutgoing(&sc, price: 100_000.0)!
        XCTAssertEqual(deducted, 500.0, accuracy: 0.01)
        XCTAssertLessThan(sc.expectedUSD.amount, 0.01)
        XCTAssertEqual(sc.backingSats, 0)
    }

    func testOutgoingZeroPriceReturnsNil() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 300_000)
        XCTAssertNil(StabilityService.reconcileOutgoing(&sc, price: 0.0))
        XCTAssertEqual(sc.expectedUSD.amount, 500.0)
    }

    func testOutgoingZeroBackingReturnsNil() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 300_000)
        sc.backingSats = 0
        XCTAssertNil(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
    }

    func testOutgoingAtDifferentPrices() {
        var sc1 = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 400_000)
        let d1 = StabilityService.reconcileOutgoing(&sc1, price: 100_000.0)!

        var sc2 = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 400_000)
        let d2 = StabilityService.reconcileOutgoing(&sc2, price: 200_000.0)!

        XCTAssertEqual(d1, 100.0, accuracy: 0.01)
        XCTAssertEqual(d2, 200.0, accuracy: 0.01)
    }

    // MARK: - reconcileForwarded

    func testForwardedCoveredByNative() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        sc.isStableReceiver = false
        XCTAssertNil(StabilityService.reconcileForwarded(&sc, userSats: 1_000_000, totalForwardedSats: 200_000, price: 100_000.0))
        XCTAssertEqual(sc.expectedUSD.amount, 500.0)
    }

    func testForwardedEatsIntoStable() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        let deducted = StabilityService.reconcileForwarded(&sc, userSats: 1_000_000, totalForwardedSats: 700_000, price: 100_000.0)!
        XCTAssertEqual(deducted, 200.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 300.0, accuracy: 0.01)
    }

    func testForwardedAllStableNoNative() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 500_000)
        let deducted = StabilityService.reconcileForwarded(&sc, userSats: 500_000, totalForwardedSats: 100_000, price: 100_000.0)!
        XCTAssertEqual(deducted, 100.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 400.0, accuracy: 0.01)
    }

    func testForwardedZeroExpectedUSD() {
        var sc = testSC(expectedUSD: 0.0, price: 100_000.0, receiverSats: 500_000)
        XCTAssertNil(StabilityService.reconcileForwarded(&sc, userSats: 500_000, totalForwardedSats: 100_000, price: 100_000.0))
    }

    func testForwardedZeroPrice() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        XCTAssertNil(StabilityService.reconcileForwarded(&sc, userSats: 1_000_000, totalForwardedSats: 700_000, price: 0.0))
    }

    // MARK: - reconcileIncoming

    func testIncomingResetsBackingToEquilibrium() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_200_000)
        sc.backingSats = 600_000
        StabilityService.reconcileIncoming(&sc)
        let expectedBacking = UInt64(500.0 / 100_000.0 * 100_000_000.0)
        XCTAssertEqual(sc.backingSats, expectedBacking)
    }

    func testIncomingNoChangeWhenAtEquilibrium() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        let backingBefore = sc.backingSats
        StabilityService.reconcileIncoming(&sc)
        XCTAssertEqual(sc.backingSats, backingBefore)
    }

    func testIncomingSkipsWhenNoStablePosition() {
        var sc = testSC(expectedUSD: 0.0, price: 100_000.0, receiverSats: 500_000)
        sc.backingSats = 12345
        StabilityService.reconcileIncoming(&sc)
        XCTAssertEqual(sc.backingSats, 12345)
    }

    func testIncomingSkipsWhenNoPrice() {
        var sc = testSC(expectedUSD: 500.0, price: 0.0, receiverSats: 500_000)
        sc.backingSats = 12345
        StabilityService.reconcileIncoming(&sc)
        XCTAssertEqual(sc.backingSats, 12345)
    }

    func testIncomingPreservesExpectedUSD() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_500_000)
        StabilityService.reconcileIncoming(&sc)
        XCTAssertEqual(sc.expectedUSD.amount, 500.0)
    }

    // MARK: - applyTrade

    func testTradeBuyReducesStable() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.applyTrade(&sc, newExpectedUSD: 300.0, price: 100_000.0)
        XCTAssertEqual(sc.expectedUSD.amount, 300.0)
        let expectedBacking = UInt64(300.0 / 100_000.0 * 100_000_000.0)
        XCTAssertEqual(sc.backingSats, expectedBacking)
    }

    func testTradeSellIncreasesStable() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.applyTrade(&sc, newExpectedUSD: 700.0, price: 100_000.0)
        XCTAssertEqual(sc.expectedUSD.amount, 700.0)
        let expectedBacking = UInt64(700.0 / 100_000.0 * 100_000_000.0)
        XCTAssertEqual(sc.backingSats, expectedBacking)
    }

    func testTradeToZero() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.applyTrade(&sc, newExpectedUSD: 0.0, price: 100_000.0)
        XCTAssertEqual(sc.expectedUSD.amount, 0.0)
        XCTAssertEqual(sc.backingSats, 0)
    }

    func testTradeZeroPriceSkipsBackingUpdate() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        let backingBefore = sc.backingSats
        StabilityService.applyTrade(&sc, newExpectedUSD: 700.0, price: 0.0)
        XCTAssertEqual(sc.expectedUSD.amount, 700.0)
        XCTAssertEqual(sc.backingSats, backingBefore)
    }

    func testTradeAtDifferentPrice() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.applyTrade(&sc, newExpectedUSD: 500.0, price: 200_000.0)
        let expectedBacking = UInt64(500.0 / 200_000.0 * 100_000_000.0)
        XCTAssertEqual(sc.backingSats, expectedBacking)
        XCTAssertEqual(expectedBacking, 250_000)
    }

    func testTradeFullBalanceToStable() {
        var sc = testSC(expectedUSD: 0.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.applyTrade(&sc, newExpectedUSD: 1000.0, price: 100_000.0)
        XCTAssertEqual(sc.expectedUSD.amount, 1000.0)
        XCTAssertEqual(sc.backingSats, 1_000_000)
    }

    // MARK: - recomputeNative

    func testNativeHalfStableHalfNative() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.recomputeNative(&sc)
        XCTAssertEqual(sc.nativeChannelBTC.sats, 500_000)
    }

    func testNativeFullyStabilized() {
        var sc = testSC(expectedUSD: 1000.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.recomputeNative(&sc)
        XCTAssertEqual(sc.nativeChannelBTC.sats, 0)
    }

    func testNativeBackingExceedsReceiverSaturates() {
        var sc = testSC(expectedUSD: 1000.0, price: 100_000.0, receiverSats: 800_000)
        StabilityService.recomputeNative(&sc)
        XCTAssertEqual(sc.nativeChannelBTC.sats, 0)
    }

    func testNativeUpdatedAfterReconcileIncoming() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_200_000)
        sc.backingSats = 600_000
        StabilityService.reconcileIncoming(&sc)
        XCTAssertEqual(sc.nativeChannelBTC.sats, 1_200_000 - 500_000)
    }

    func testNativeUpdatedAfterApplyTrade() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        StabilityService.applyTrade(&sc, newExpectedUSD: 800.0, price: 100_000.0)
        let expectedBacking = UInt64(800.0 / 100_000.0 * 100_000_000.0)
        XCTAssertEqual(sc.nativeChannelBTC.sats, 1_000_000 - expectedBacking)
    }

    func testNativeUpdatedAfterReconcileOutgoing() {
        var sc = testSC(expectedUSD: 1000.0, price: 100_000.0, receiverSats: 900_000)
        _ = StabilityService.reconcileOutgoing(&sc, price: 100_000.0)
        XCTAssertLessThanOrEqual(sc.nativeChannelBTC.sats, 1)
    }

    // MARK: - Bitcoin / USD

    func testBitcoinFromSats() {
        let btc = Bitcoin.fromSats(100_000_000)
        XCTAssertEqual(btc.toBTC(), 1.0)
    }

    func testBitcoinFromBTC() {
        let btc = Bitcoin.fromBTC(1.5)
        XCTAssertEqual(btc.sats, 150_000_000)
    }

    func testBitcoinFromUSD() {
        let usd = USD(amount: 100_000.0)
        let btc = Bitcoin.fromUSD(usd, price: 100_000.0)
        XCTAssertEqual(btc.toBTC(), 1.0)
    }

    func testUSDFromBitcoin() {
        let btc = Bitcoin.fromBTC(1.0)
        let usd = USD.fromBitcoin(btc, price: 50_000.0)
        XCTAssertEqual(usd.amount, 50_000.0)
    }

    func testUSDToMsats() {
        let usd = USD(amount: 100.0)
        let msats = usd.toMsats(price: 100_000.0)
        XCTAssertEqual(msats, 100_000_000)
    }

    // MARK: - Stability Check Action

    func testStabilityActionStable() {
        let sc = testSC(expectedUSD: 100.0, price: 100_000.0, receiverSats: 100_000)
        let result = StabilityService.checkStabilityAction(sc, price: 100_000.0)
        XCTAssertEqual(result.action, .stable)
    }

    func testStabilityActionPay() {
        // Price went up — stable portion is worth more → need to pay
        var sc = testSC(expectedUSD: 100.0, price: 100_000.0, receiverSats: 200_000)
        sc.isStableReceiver = true
        // At $200k, backing sats (100k) now worth $200 vs target $100 → 100% deviation
        let result = StabilityService.checkStabilityAction(sc, price: 200_000.0)
        XCTAssertEqual(result.action, .pay)
    }
}
