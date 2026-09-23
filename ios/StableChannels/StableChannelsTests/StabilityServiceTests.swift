import XCTest
@testable import StableChannels

final class StabilityServiceTests: XCTestCase {
    // MARK: - Helper

    private func testSC(expectedUSD: Double, price: Double, receiverSats: UInt64) -> StableChannel {
        let backing: UInt64 = price > 0
            ? UInt64(round(expectedUSD / price * 100_000_000.0))
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

    func testOutgoingEatsIntoStable() throws {
        var sc = testSC(expectedUSD: 1000.0, price: 100_000.0, receiverSats: 900_000)
        let deducted = StabilityService.reconcileOutgoing(&sc, price: 100_000.0)
        XCTAssertNotNil(deducted)
        XCTAssertEqual(try XCTUnwrap(deducted), 100.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 900.0, accuracy: 0.01)
        let expectedBacking = UInt64(round(900.0 / 100_000.0 * 100_000_000.0))
        XCTAssertEqual(sc.backingSats, expectedBacking)
    }

    func testReconcileOutgoingIsIdempotentBelowPar() throws {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 100.0)
        sc.backingSats = 90_000
        sc.stableReceiverBTC = Bitcoin(sats: 82_000)
        sc.isStableReceiver = true

        let deducted = try XCTUnwrap(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
        XCTAssertEqual(deducted, 8.0, accuracy: 0.0001)
        XCTAssertEqual(sc.expectedUSD.amount, 92.0, accuracy: 0.0001)
        XCTAssertEqual(sc.backingSats, 82_000)

        let deductedAgain = StabilityService.reconcileOutgoing(&sc, price: 100_000.0)
        XCTAssertNil(deductedAgain)
        XCTAssertEqual(sc.expectedUSD.amount, 92.0, accuracy: 0.0001)
        XCTAssertEqual(sc.backingSats, 82_000)
    }

    func testReconcileOutgoingPreservesBackingAtZeroBoundary() throws {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 10.0)
        sc.backingSats = 20_000
        sc.stableReceiverBTC = Bitcoin(sats: 5_000)
        sc.isStableReceiver = true

        // Overflow is 15_000 sats = $15, but target is only $10. Report the target drop only.
        let deducted = try XCTUnwrap(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
        XCTAssertEqual(deducted, 10.0, accuracy: 0.0001)
        XCTAssertEqual(sc.expectedUSD.amount, 0.0, accuracy: 0.0001)
        // Backing stays at receiverSats: the residue is an unsettled LSP surplus
        XCTAssertEqual(sc.backingSats, 5_000)
        XCTAssertEqual(sc.nativeChannelBTC.sats, 0)
    }

    func testOutgoingPartialStableDeduction() throws {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 300_000)
        let deducted = try XCTUnwrap(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
        XCTAssertEqual(deducted, 200.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 300.0, accuracy: 0.01)
    }

    func testOutgoingSpendsEntireStable() throws {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 0)
        let deducted = try XCTUnwrap(StabilityService.reconcileOutgoing(&sc, price: 100_000.0))
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

    func testOutgoingAtDifferentPrices() throws {
        var sc1 = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 400_000)
        let d1 = try XCTUnwrap(StabilityService.reconcileOutgoing(&sc1, price: 100_000.0))

        var sc2 = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 400_000)
        let d2 = try XCTUnwrap(StabilityService.reconcileOutgoing(&sc2, price: 200_000.0))

        XCTAssertEqual(d1, 100.0, accuracy: 0.01)
        XCTAssertEqual(d2, 200.0, accuracy: 0.01)
    }

    // MARK: - reconcileForwarded

    func testForwardedCoveredByNative() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        sc.isStableReceiver = false
        XCTAssertNil(StabilityService.reconcileForwarded(
            &sc,
            userSats: 1_000_000,
            totalForwardedSats: 200_000,
            price: 100_000.0
        ))
        XCTAssertEqual(sc.expectedUSD.amount, 500.0)
    }

    func testForwardedEatsIntoStable() throws {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        let deducted = try XCTUnwrap(StabilityService.reconcileForwarded(
            &sc,
            userSats: 1_000_000,
            totalForwardedSats: 700_000,
            price: 100_000.0
        ))
        XCTAssertEqual(deducted, 200.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 300.0, accuracy: 0.01)
    }

    func testForwardedAllStableNoNative() throws {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 500_000)
        let deducted = try XCTUnwrap(StabilityService.reconcileForwarded(
            &sc,
            userSats: 500_000,
            totalForwardedSats: 100_000,
            price: 100_000.0
        ))
        XCTAssertEqual(deducted, 100.0, accuracy: 0.01)
        XCTAssertEqual(sc.expectedUSD.amount, 400.0, accuracy: 0.01)
    }

    func testForwardedZeroExpectedUSD() {
        var sc = testSC(expectedUSD: 0.0, price: 100_000.0, receiverSats: 500_000)
        XCTAssertNil(StabilityService.reconcileForwarded(
            &sc,
            userSats: 500_000,
            totalForwardedSats: 100_000,
            price: 100_000.0
        ))
    }

    func testForwardedZeroPrice() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_000_000)
        XCTAssertNil(StabilityService.reconcileForwarded(
            &sc,
            userSats: 1_000_000,
            totalForwardedSats: 700_000,
            price: 0.0
        ))
    }

    // MARK: - reconcileIncoming

    func testIncomingPreservesBackingSats() {
        var sc = testSC(expectedUSD: 500.0, price: 100_000.0, receiverSats: 1_200_000)
        sc.backingSats = 600_000
        StabilityService.reconcileIncoming(&sc)
        XCTAssertEqual(sc.backingSats, 600_000)
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
        XCTAssertEqual(sc.nativeChannelBTC.sats, 1_200_000 - 600_000)
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

    // MARK: - StabilityFreshness (chain-freshness gate for stability sends)

    private let freshnessNow: UInt64 = 1_000_000

    func testMissingSyncTimestampBlocksSend() {
        XCTAssertFalse(StabilityFreshness.isFresh(nil, now: freshnessNow))
        XCTAssertNil(StabilityFreshness.syncAgeSecs(nil, now: freshnessNow))
    }

    func testFutureSyncTimestampBlocksSend() {
        XCTAssertFalse(StabilityFreshness.isFresh(freshnessNow + 1, now: freshnessNow))
        XCTAssertNil(StabilityFreshness.syncAgeSecs(freshnessNow + 1, now: freshnessNow))
    }

    func testSyncTimestampOlderThanWindowBlocksSend() {
        let maxAge = Constants.stabilityMaxLightningSyncAgeSecs
        XCTAssertFalse(StabilityFreshness.isFresh(freshnessNow - maxAge - 1, now: freshnessNow))
    }

    func testSyncTimestampExactlyAtWindowBoundaryIsAccepted() {
        let maxAge = Constants.stabilityMaxLightningSyncAgeSecs
        XCTAssertTrue(StabilityFreshness.isFresh(freshnessNow - maxAge, now: freshnessNow))
    }

    func testFreshSyncTimestampIsAccepted() {
        XCTAssertTrue(StabilityFreshness.isFresh(freshnessNow, now: freshnessNow))
        XCTAssertTrue(StabilityFreshness.isFresh(freshnessNow - 1, now: freshnessNow))
        XCTAssertEqual(StabilityFreshness.syncAgeSecs(freshnessNow - 90, now: freshnessNow), 90)
    }

    // MARK: - repairBooksAboveLiveBalance

    func testRepairBooksReturnsNilWhenBackingDoesNotExceedReceiver() {
        var channel = StableChannel.default
        channel.stableReceiverBTC = Bitcoin(sats: 100_000)
        channel.backingSats = 100_000
        channel.expectedUSD = USD(amount: 100.0)

        let result = StabilityService.repairBooksAboveLiveBalance(&channel, price: 100_000)
        XCTAssertNil(result)
        XCTAssertEqual(channel.backingSats, 100_000)
        XCTAssertEqual(channel.expectedUSD.amount, 100.0)
    }

    func testRepairBooksReturnsNilWhenPriceIsZeroOrNegative() {
        var channel = StableChannel.default
        channel.stableReceiverBTC = Bitcoin(sats: 50_000)
        channel.backingSats = 100_000
        channel.expectedUSD = USD(amount: 100.0)

        let resultZero = StabilityService.repairBooksAboveLiveBalance(&channel, price: 0.0)
        XCTAssertNil(resultZero)

        let resultNegative = StabilityService.repairBooksAboveLiveBalance(&channel, price: -50_000)
        XCTAssertNil(resultNegative)
    }

    func testRepairBooksDeductsOverflowAndClampsBacking() {
        var channel = StableChannel.default
        channel.stableReceiverBTC = Bitcoin(sats: 60_000)
        channel.backingSats = 100_000
        channel.expectedUSD = USD(amount: 100.0)
        let price = 100_000.0 // 1 sat = $0.001

        let result = StabilityService.repairBooksAboveLiveBalance(&channel, price: price)
        XCTAssertNotNil(result)
        XCTAssertEqual(result?.overflowSats, 40_000)
        XCTAssertEqual(result?.usdDeducted, 40.0)
        XCTAssertEqual(result?.oldExpectedUSD, 100.0)
        XCTAssertEqual(result?.newExpectedUSD, 60.0)

        XCTAssertEqual(channel.backingSats, 60_000)
        XCTAssertEqual(channel.expectedUSD.amount, 60.0)
        XCTAssertEqual(channel.nativeSats, 0)
    }

    func testRepairBooksPreservesBackingAtZeroBoundary() throws {
        var channel = StableChannel.default
        channel.stableReceiverBTC = Bitcoin(sats: 10)
        channel.backingSats = 100_000
        channel.expectedUSD = USD(amount: 10.0)
        let price = 100_000.0 // 99_990 overflow sats = $99.99, but target is only $10

        let result = StabilityService.repairBooksAboveLiveBalance(&channel, price: price)
        let unwrapped = try XCTUnwrap(result)
        XCTAssertEqual(unwrapped.overflowSats, 99_990)
        // usdDeducted is only the target drop, not the full overflow
        XCTAssertEqual(unwrapped.usdDeducted, 10.0, accuracy: 0.0001)
        // Backing stays at receiverSats: the residue is unsettled LSP surplus
        XCTAssertEqual(channel.backingSats, 10)
        XCTAssertEqual(channel.expectedUSD.amount, 0.0)
        XCTAssertEqual(channel.nativeSats, 0)
    }

    // MARK: - checkStabilityAction deadband escape

    func testZeroTargetWithBackingIsNotClosedPosition() {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 0.0)
        sc.backingSats = 5_000
        sc.stableReceiverBTC = Bitcoin(sats: 10_000)
        sc.isStableReceiver = true

        // Sub-cent target + backing should NOT bail as "stable" -- the surplus must settle.
        let result = StabilityService.checkStabilityAction(sc, price: 100_000.0)
        XCTAssertNotEqual(result.action, .stable,
                          "Zero target with backing should not be treated as closed")
        // The backing (5000 sats = $5) is above the $0 target, so action should be .pay
        XCTAssertEqual(result.action, .pay)
    }

    func testZeroTargetWithZeroBackingIsClosedPosition() {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 0.0)
        sc.backingSats = 0
        sc.stableReceiverBTC = Bitcoin(sats: 10_000)
        sc.isStableReceiver = true

        let result = StabilityService.checkStabilityAction(sc, price: 100_000.0)
        XCTAssertEqual(result.action, .stable)
    }

    func testPercentFromParClampedAtZeroTarget() {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 0.001) // Sub-cent but still with backing
        sc.backingSats = 1_000
        sc.stableReceiverBTC = Bitcoin(sats: 2_000)
        sc.isStableReceiver = true

        let result = StabilityService.checkStabilityAction(sc, price: 100_000.0)
        // percentFromPar should be large enough to escape the deadband
        XCTAssertGreaterThan(result.percentFromPar, 1.0)
    }

    // MARK: - spendConsumesLspSurplus

    func testSpendCoveredByNativeDoesNotConsumeSurplus() {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 50.0)
        sc.backingSats = 60_000 // $60 backing vs $50 target = above par
        sc.stableReceiverBTC = Bitcoin(sats: 100_000)
        sc.isStableReceiver = true

        // Native = 100_000 - 60_000 = 40_000 sats. Spending 30_000 is fully native.
        XCTAssertFalse(
            StabilityService.spendConsumesLspSurplus(sc, price: 100_000.0, amountSats: 30_000)
        )
    }

    func testSpendExceedingTargetConsumesSurplus() {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 50.0)
        sc.backingSats = 60_000 // $60 backing vs $50 target = $10 surplus owed to LSP
        sc.stableReceiverBTC = Bitcoin(sats: 100_000)
        sc.isStableReceiver = true

        // Native = 40_000. Spending 95_000 overflows into 55_000 sats of backing = $55 > $50 target
        XCTAssertTrue(
            StabilityService.spendConsumesLspSurplus(sc, price: 100_000.0, amountSats: 95_000)
        )
    }

    func testSpendNotAboveParReturnsFalse() {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 100.0)
        sc.backingSats = 80_000 // $80 backing vs $100 target = below par, not .pay
        sc.stableReceiverBTC = Bitcoin(sats: 100_000)
        sc.isStableReceiver = true

        // Position is below par so there is no LSP surplus to consume
        XCTAssertFalse(
            StabilityService.spendConsumesLspSurplus(sc, price: 100_000.0, amountSats: 99_000)
        )
    }
}
