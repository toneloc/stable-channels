import XCTest
import LDKNode
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

    func testBitcoinFromUSDZeroOrInvalidPrice() {
        let usd = USD(amount: 100.0)
        XCTAssertEqual(Bitcoin.fromUSD(usd, price: 0.0).sats, 0)
        XCTAssertEqual(Bitcoin.fromUSD(usd, price: -50_000.0).sats, 0)
        XCTAssertEqual(Bitcoin.fromUSD(usd, price: Double.nan).sats, 0)
        XCTAssertEqual(Bitcoin.fromUSD(usd, price: Double.infinity).sats, 0)
    }

    func testBitcoinFromBTCInvalidValues() {
        XCTAssertEqual(Bitcoin.fromBTC(0.0).sats, 0)
        XCTAssertEqual(Bitcoin.fromBTC(-1.5).sats, 0)
        XCTAssertEqual(Bitcoin.fromBTC(Double.nan).sats, 0)
        XCTAssertEqual(Bitcoin.fromBTC(Double.infinity).sats, 0)
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

    // MARK: - AppState Tick & Spend Guard Tests

    @MainActor
    func testAppStateEvaluateStabilityActionAtZeroTargetWithBacking() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 5_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 10_000)

        let result = appState.evaluateStabilityAction(price: 100_000.0, hasChannels: true)
        XCTAssertNotNil(result)
        XCTAssertEqual(result?.action, .pay)
    }

    @MainActor
    func testAppStateEvaluateStabilityActionAtZeroTargetWithoutBacking() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 0
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 10_000)

        let result = appState.evaluateStabilityAction(price: 100_000.0, hasChannels: true)
        XCTAssertNil(result)
    }

    @MainActor
    func testAppStateEvaluateStabilityActionWithoutChannelsReturnsNil() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 5_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 10_000)

        let result = appState.evaluateStabilityAction(price: 100_000.0, hasChannels: false)
        XCTAssertNil(result)
    }

    @MainActor
    func testEnsureNoUnsettledSurplusThrowsWhenConsumingSurplus() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 50.0)
        appState.stableChannel.backingSats = 60_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)

        XCTAssertThrowsError(try appState.ensureNoUnsettledSurplus(amountMsat: 95_000_000, price: 100_000.0)) { error in
            let nsError = error as NSError
            XCTAssertTrue(nsError.localizedDescription.contains("still settling"))
            if case .surplusSettling(let owedUSD) = error as? StabilitySpendError {
                XCTAssertEqual(owedUSD, 10.0, accuracy: 0.01)
            } else {
                XCTFail("Expected StabilitySpendError.surplusSettling, got: \(error)")
            }
        }
    }

    @MainActor
    func testEnsureNoUnsettledSurplusAllowsNativeSpend() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 50.0)
        appState.stableChannel.backingSats = 60_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)

        XCTAssertNoThrow(try appState.ensureNoUnsettledSurplus(amountMsat: 30_000_000, price: 100_000.0))
    }

    @MainActor
    func testEnsureNoUnsettledSurplusAtZeroTargetWithBacking() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 5_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 5_000)

        XCTAssertThrowsError(try appState.ensureNoUnsettledSurplus(amountSats: 1_000, price: 100_000.0)) { error in
            let nsError = error as NSError
            XCTAssertTrue(nsError.localizedDescription.contains("still settling"))
            if case .surplusSettling(let owedUSD) = error as? StabilitySpendError {
                XCTAssertEqual(owedUSD, 5.0, accuracy: 0.01)
            } else {
                XCTFail("Expected StabilitySpendError.surplusSettling, got: \(error)")
            }
        }
    }

    @MainActor
    func testEnsureNoUnsettledSurplusFailsOpenWhenPriceMissing() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 50.0)
        appState.stableChannel.backingSats = 60_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)

        // Missing/zero price fails open without throwing
        XCTAssertNoThrow(try appState.ensureNoUnsettledSurplus(amountMsat: 95_000_000, price: 0.0))
    }

    @MainActor
    func testEnsureNoUnsettledSurplusFailsOpenWhenNextOutboundHtlcLimitIsZero() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 50.0)
        appState.stableChannel.backingSats = 60_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)

        let details = makeMockChannel(
            userChannelId: "test-channel",
            outboundCapacityMsat: 100_000_000,
            nextOutboundHtlcLimitMsat: 0
        )
        appState.nodeService.channelsOverride = [details]
        defer { appState.nodeService.channelsOverride = nil }

        // When nextOutboundHtlcLimitMsat is 0, min(outbound, nextHtlc) == 0, failing open to prevent trapped funds
        XCTAssertNoThrow(try appState.ensureNoUnsettledSurplus(amountMsat: 95_000_000, price: 100_000.0))
    }

    @MainActor
    func testRepairBooksAboveLiveBalanceSkippedWhenPendingOutgoingPaymentExists() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let appState = AppState()
        let db = try DatabaseService(dataDir: tempDir)
        appState.databaseService = db
        appState.priceService.setPriceForTesting(100_000.0)
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.channelId = "test-channel"
        appState.hasReadyChannel = true
        appState.stableChannel.expectedUSD = USD(amount: 100.0)
        appState.stableChannel.backingSats = 100_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 60_000)

        // Record a pending outgoing payment (in-flight HTLC)
        _ = try db.paymentRepo.recordPayment(
            paymentId: "pending-htlc-1",
            paymentType: "lightning",
            direction: "sent",
            amountMsat: 40_000_000,
            amountUSD: 40.0,
            btcPrice: 100_000.0,
            counterparty: nil,
            status: "pending"
        )

        appState.repairBooksAboveLiveBalance()

        // Books should NOT be modified because a payment is in-flight
        XCTAssertEqual(appState.stableChannel.expectedUSD.amount, 100.0)
        XCTAssertEqual(appState.stableChannel.backingSats, 100_000)

        // Mark payment failed (HTLC canceled)
        try db.paymentRepo.updatePaymentStatus(paymentId: "pending-htlc-1", status: "failed")
        XCTAssertFalse(try db.paymentRepo.hasPendingOutgoingPayment())

        // If the balance genuinely remained low without an in-flight payment, repair runs
        appState.repairBooksAboveLiveBalance()
        XCTAssertEqual(appState.stableChannel.expectedUSD.amount, 60.0)
        XCTAssertEqual(appState.stableChannel.backingSats, 60_000)
    }

    @MainActor
    func testDetectOnchainDepositAbsorbsWhenChannelClosing() {
        let appState = AppState()
        appState.isChannelClosing = true
        appState.prevOnchainSats = 5_000
        appState.onchainBalanceSats = 105_000

        appState.detectOnchainDeposit()
        XCTAssertEqual(appState.prevOnchainSats, 105_000)
    }

    @MainActor
    func testDetectOnchainDepositAbsorbsWhenPendingCloseOpExists() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)
        let appState = AppState()
        appState.databaseService = dbService
        _ = dbService.pendingOpRepo.insertPendingOperation(
            opId: "close-test-ch",
            opType: "channel_close",
            fundingOutpointTxid: "txid-123",
            fundingOutpointVout: 0
        )
        appState.isChannelClosing = false
        appState.prevOnchainSats = 5_000
        appState.onchainBalanceSats = 105_000

        appState.detectOnchainDeposit()
        XCTAssertEqual(appState.prevOnchainSats, 105_000)
    }

    @MainActor
    func testDetectOnchainDepositAbsorbsWhenMatchingClosePaymentExists() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)
        let appState = AppState()
        appState.databaseService = dbService
        _ = try dbService.paymentRepo.recordPayment(
            paymentId: "close-payment-1",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: 100_000_000,
            amountUSD: 100.0,
            btcPrice: 100_000.0,
            counterparty: nil,
            status: "completed"
        )
        appState.isChannelClosing = false
        appState.prevOnchainSats = 5_000
        // Sweep of 99,000 sats confirms (1,000 sat mining fee difference)
        appState.onchainBalanceSats = 104_000

        appState.detectOnchainDeposit()
        XCTAssertEqual(appState.prevOnchainSats, 104_000)

        // Verify no duplicate onchain payment row was created
        let payments = try dbService.paymentRepo.getRecentPayments(limit: 50)
        let onchainRows = payments.filter { $0.paymentType == "onchain" }
        XCTAssertTrue(onchainRows.isEmpty)
    }

    @MainActor
    func testCalculateSettlementAmountMsatCapsToOutboundCapacity() {
        let appState = AppState()

        // 100 USD at $100k/BTC = 100,000 sats = 100,000,000 msat.
        // With 75,000 sats outbound capacity (25,000 sat reserve), payment is capped to 75,000,000 msat.
        let capped = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 100.0,
            price: 100_000.0,
            outboundCapacityMsat: 75_000_000
        )
        XCTAssertEqual(capped, 75_000_000)

        // Within capacity: 50 USD = 50,000 sats = 50,000,000 msat.
        let withinCapacity = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 50.0,
            price: 100_000.0,
            outboundCapacityMsat: 75_000_000
        )
        XCTAssertEqual(withinCapacity, 50_000_000)

        // Zero outbound capacity returns 0.
        let zeroCapacity = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 100.0,
            price: 100_000.0,
            outboundCapacityMsat: 0
        )
        XCTAssertEqual(zeroCapacity, 0)
    }

    @MainActor
    func testCalculateSettlementAmountMsatCapsToNextOutboundHtlcLimit() {
        let appState = AppState()

        // 100 USD at $100k/BTC = 100,000 sats = 100,000,000 msat.
        // Outbound capacity is 75,000,000 msat, but nextOutboundHtlcLimitMsat is 50,000,000 msat.
        // Must be capped to 50,000,000 msat.
        let capped = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 100.0,
            price: 100_000.0,
            outboundCapacityMsat: 75_000_000,
            nextOutboundHtlcLimitMsat: 50_000_000
        )
        XCTAssertEqual(capped, 50_000_000)

        // When nextOutboundHtlcLimitMsat is higher than outboundCapacityMsat, outboundCapacity binds.
        let capacityBinds = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 100.0,
            price: 100_000.0,
            outboundCapacityMsat: 60_000_000,
            nextOutboundHtlcLimitMsat: 80_000_000
        )
        XCTAssertEqual(capacityBinds, 60_000_000)
    }

    @MainActor
    func testEnsureNoUnsettledSurplusReleasesWhenOutboundCapacityIsZero() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 5_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 5_000)

        // When outbound capacity is available, guard blocks spend
        XCTAssertThrowsError(
            try appState.ensureNoUnsettledSurplus(
                amountSats: 1_000,
                price: 100_000.0,
                outboundCapacityMsat: 5_000_000
            )
        )

        // When only unspendable reserve remains (outboundCapacity = 0), guard releases so funds are not trapped
        XCTAssertNoThrow(
            try appState.ensureNoUnsettledSurplus(
                amountSats: 1_000,
                price: 100_000.0,
                outboundCapacityMsat: 0
            )
        )
    }

    @MainActor
    func testCalculateSettlementAmountMsatSubSatAndRoundingEdgeCases() {
        let appState = AppState()

        // Sub-sat dollar amount ($0.0000001 at $100k/BTC = 0.0001 sat) floors to 0 msat
        let subSat = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 0.0000001,
            price: 100_000.0,
            outboundCapacityMsat: 100_000_000
        )
        XCTAssertEqual(subSat, 0)

        // Fractional msat capacity (75,999 msat = 75.999 sats) floors to whole sat boundary (75,000 msat)
        let fractionalCapacity = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 100.0,
            price: 100_000.0,
            outboundCapacityMsat: 75_999
        )
        XCTAssertEqual(fractionalCapacity, 75_000)

        // Zero or negative price returns 0 msat
        let zeroPrice = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 50.0,
            price: 0.0,
            outboundCapacityMsat: 100_000_000
        )
        XCTAssertEqual(zeroPrice, 0)

        let negativePrice = appState.calculateSettlementAmountMsat(
            dollarsFromPar: 50.0,
            price: -100_000.0,
            outboundCapacityMsat: 100_000_000
        )
        XCTAssertEqual(negativePrice, 0)
    }

    func testStabilityServiceCalculateSettlementAmountMsatPureFunctionalCore() {
        // Pure calculation with no AppState or nodeService dependency
        let amount = StabilityService.calculateSettlementAmountMsat(
            dollarsFromPar: 100.0,
            price: 100_000.0,
            outboundCapacityMsat: 75_000_000,
            nextOutboundHtlcLimitMsat: 50_000_000
        )
        XCTAssertEqual(amount, 50_000_000)

        // When capacity is higher than needed, uncapped binds
        let uncappedBinds = StabilityService.calculateSettlementAmountMsat(
            dollarsFromPar: 20.0,
            price: 100_000.0,
            outboundCapacityMsat: 75_000_000,
            nextOutboundHtlcLimitMsat: 50_000_000
        )
        XCTAssertEqual(uncappedBinds, 20_000_000)

        // Non-positive price or 0 dollarsFromPar returns 0
        XCTAssertEqual(StabilityService.calculateSettlementAmountMsat(dollarsFromPar: 0.0, price: 100_000.0), 0)
        XCTAssertEqual(StabilityService.calculateSettlementAmountMsat(dollarsFromPar: 10.0, price: 0.0), 0)
    }

    @MainActor
    func testEnsureNoUnsettledSurplusExactTargetBoundary() {
        let appState = AppState()
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 50.0)
        appState.stableChannel.backingSats = 60_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)

        // Native balance = 40,000 sats.
        // Target balance = $50 @ $100k/BTC = 50,000 sats.
        // Total allowed spend before touching surplus = 40,000 + 50,000 = 90,000 sats.

        // Exactly 90,000 sats: overflow is 50,000 sats ($50.00), which equals target USD.
        // The guard condition is strictly greater (overflowUsd > expectedUSD), so this spend passes.
        XCTAssertNoThrow(
            try appState.ensureNoUnsettledSurplus(amountSats: 90_000, price: 100_000.0)
        )

        // 90,001 sats: overflow is 50,001 sats ($50.001), which exceeds target USD into LSP surplus.
        // The guard must block this spend.
        XCTAssertThrowsError(
            try appState.ensureNoUnsettledSurplus(amountSats: 90_001, price: 100_000.0)
        )
    }

    @MainActor
    func testEnsureNoUnsettledSurplusSkipsForProviderAndEmptyChannel() {
        let appState = AppState()

        // Provider position (not stable receiver): surplus guard never applies
        appState.stableChannel.isStableReceiver = false
        appState.stableChannel.userChannelId = "test-channel"
        appState.stableChannel.expectedUSD = USD(amount: 50.0)
        appState.stableChannel.backingSats = 60_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)
        XCTAssertNoThrow(try appState.ensureNoUnsettledSurplus(amountSats: 95_000, price: 100_000.0))

        // Empty userChannelId: no active channel, guard returns without throwing
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = ""
        XCTAssertNoThrow(try appState.ensureNoUnsettledSurplus(amountSats: 95_000, price: 100_000.0))
    }

    @MainActor
    func testDetectOnchainDepositDefersWhenSweepingOrSplicePending() {
        let appState = AppState()
        appState.prevOnchainSats = 5_000
        appState.onchainBalanceSats = 100_000

        // When isSweeping is true, deposit detection defers and prevOnchainSats is NOT advanced
        appState.isSweeping = true
        appState.detectOnchainDeposit()
        XCTAssertEqual(appState.prevOnchainSats, 5_000)

        // When pendingSplice exists, deposit detection defers and prevOnchainSats is NOT advanced
        appState.isSweeping = false
        appState.pendingSplice = PendingSplice(
            direction: "out",
            amountSats: 20_000,
            address: "tb1qtestaddress"
        )
        appState.detectOnchainDeposit()
        XCTAssertEqual(appState.prevOnchainSats, 5_000)
    }

    @MainActor
    func testDetectOnchainDepositDustFluctuationIgnored() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)
        let appState = AppState()
        appState.databaseService = dbService
        appState.prevOnchainSats = 5_000
        // Increase of only 500 sats (< 1000 sats threshold)
        appState.onchainBalanceSats = 5_500

        appState.detectOnchainDeposit()
        // Baseline advances to avoid re-triggering
        XCTAssertEqual(appState.prevOnchainSats, 5_500)

        // No onchain payment row recorded
        let payments = try dbService.paymentRepo.getRecentPayments(limit: 50)
        XCTAssertTrue(payments.isEmpty)
    }

    // MARK: - USD.toMsats Overflow Boundary

    func testUSDToMsatsOverflowBoundaryAtMaxDouble() {
        // Double(UInt64.max) is 2^64 (18446744073709551616.0).
        // A huge USD amount that evaluates to >= 2^64 millisats must clamp to UInt64.max rather than trapping.
        let hugeUSD = USD(amount: 1.0e18)
        let msats = hugeUSD.toMsats(price: 1.0)
        XCTAssertEqual(msats, UInt64.max)

        // Exact boundary check: amount / price * 1e11 = 2^64
        let twoToThe64 = 18_446_744_073_709_551_616.0
        let boundaryAmount = twoToThe64 / 100_000_000.0 / 1000.0 * 100_000.0
        let boundaryUSD = USD(amount: boundaryAmount)
        XCTAssertEqual(boundaryUSD.toMsats(price: 100_000.0), UInt64.max)
    }

    // MARK: - Mock ChannelDetails Helper

    private func makeMockChannel(
        channelId: String = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
        userChannelId: String = "test-chan-1",
        outboundCapacityMsat: UInt64,
        nextOutboundHtlcLimitMsat: UInt64? = nil,
        unspendablePunishmentReserve: UInt64 = 25_000,
        channelValueSats: UInt64 = 200_000,
        isChannelReady: Bool = true
    ) -> ChannelDetails {
        ChannelDetails(
            channelId: channelId,
            counterpartyNodeId: "020202020202020202020202020202020202020202020202020202020202020202",
            fundingTxo: nil,
            fundingRedeemScript: nil,
            shortChannelId: nil,
            outboundScidAlias: nil,
            inboundScidAlias: nil,
            channelValueSats: channelValueSats,
            unspendablePunishmentReserve: unspendablePunishmentReserve,
            userChannelId: userChannelId,
            feerateSatPer1000Weight: 253,
            outboundCapacityMsat: outboundCapacityMsat,
            inboundCapacityMsat: 100_000_000,
            confirmationsRequired: 1,
            confirmations: 6,
            isOutbound: true,
            isChannelReady: isChannelReady,
            isUsable: isChannelReady,
            isAnnounced: false,
            cltvExpiryDelta: 144,
            counterpartyUnspendablePunishmentReserve: 25_000,
            counterpartyOutboundHtlcMinimumMsat: 1_000,
            counterpartyOutboundHtlcMaximumMsat: 200_000_000,
            counterpartyForwardingInfoFeeBaseMsat: 1_000,
            counterpartyForwardingInfoFeeProportionalMillionths: 100,
            counterpartyForwardingInfoCltvExpiryDelta: 144,
            nextOutboundHtlcLimitMsat: nextOutboundHtlcLimitMsat ?? outboundCapacityMsat,
            nextOutboundHtlcMinimumMsat: 1_000,
            forceCloseSpendDelay: 144,
            inboundHtlcMinimumMsat: 1_000,
            inboundHtlcMaximumMsat: 200_000_000,
            config: ChannelConfig(
                forwardingFeeProportionalMillionths: 100,
                forwardingFeeBaseMsat: 1000,
                cltvExpiryDelta: 144,
                maxDustHtlcExposure: .fixedLimit(limitMsat: 5_000_000),
                forceCloseAvoidanceMaxFeeSatoshis: 10_000,
                acceptUnderpayingHtlcs: false
            ),
            channelShutdownState: nil
        )
    }

    @MainActor
    func testCalculateSettlementAmountMsatSelectsMatchingUserChannel() {
        let appState = AppState()
        appState.stableChannel.userChannelId = "target-chan"

        let otherChan = makeMockChannel(
            channelId: "other-chan-id",
            userChannelId: "other-chan",
            outboundCapacityMsat: 20_000_000
        )
        let targetChan = makeMockChannel(
            channelId: "target-chan-id",
            userChannelId: "target-chan",
            outboundCapacityMsat: 50_000_000
        )

        appState.nodeService.channelsOverride = [otherChan, targetChan]

        // Dollars from par: $100 at $100k/BTC = 100,000,000 msat.
        // Should select targetChan (50_000_000 msat capacity), NOT otherChan (20_000_000 msat)
        let amount = appState.calculateSettlementAmountMsat(dollarsFromPar: 100.0, price: 100_000.0)
        XCTAssertEqual(amount, 50_000_000)
    }

    @MainActor
    func testRunStabilityCheckZeroTargetCappedWithReserveSettlesSuccessfully() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)

        let appState = AppState()
        appState.databaseService = dbService
        appState.priceService.setPriceForTesting(100_000.0)

        // Zero-target position with 100,000 backing sats residue (above-par surplus)
        let channelId = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
        let userChannelId = "test-chan-1"
        let counterparty = "020202020202020202020202020202020202020202020202020202020202020202"

        appState.stableChannel.channelId = channelId
        appState.stableChannel.userChannelId = userChannelId
        appState.stableChannel.counterparty = counterparty
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 100_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)
        appState.stableChannel.lastStabilityPayment = 0
        appState.saveChannelToDB()

        // Ready channel with 75,000 sats spendable capacity and 25,000 sat reserve.
        // Total balance is 100,000 sats, but only 75,000 sats (75,000,000 msat) can be sent.
        let channel = makeMockChannel(
            channelId: channelId,
            userChannelId: userChannelId,
            outboundCapacityMsat: 75_000_000,
            unspendablePunishmentReserve: 25_000,
            channelValueSats: 200_000,
            isChannelReady: true
        )
        appState.nodeService.channelsOverride = [channel]
        appState.nodeService.lightningSyncAgeSecsOverride = 10
        appState.nodeService.signMessageOverride = { _ in "mock-signature" }

        var paymentSentMsat: UInt64?
        var paymentSentTo: PublicKey?
        appState.nodeService.sendStabilityPaymentOverride = { amountMsat, toNode, _ in
            paymentSentMsat = amountMsat
            paymentSentTo = toNode
            return "mock-payment-id-123"
        }

        // Execute runStabilityCheck
        appState.runStabilityCheck()

        // Verify: settlement was capped to 75,000,000 msat, post-claim recheck matched,
        // and payment was successfully sent!
        XCTAssertEqual(paymentSentMsat, 75_000_000)
        XCTAssertEqual(paymentSentTo, counterparty)

        // Verify happy path post-conditions:
        // 1. Payment recorded in paymentRepo
        let recorded = dbService.paymentRepo.payment(paymentId: "mock-payment-id-123")
        XCTAssertNotNil(recorded)
        XCTAssertEqual(recorded?.amountMsat, 75_000_000)
        XCTAssertEqual(recorded?.paymentType, "stability")
        XCTAssertEqual(recorded?.direction, "sent")

        // 2. Channel backing was decremented by 75,000 sats in DB (from 100,000 to 25,000 sats)
        let updatedChannel = try dbService.channelRepo.loadChannel(userChannelId: userChannelId)
        XCTAssertEqual(updatedChannel?.backingSats, 25_000)

        // 3. Pending send marker was successfully cleared upon DB commit
        XCTAssertNil(dbService.stabilityRepo.loadPendingSend())
        XCTAssertTrue(appState.stableChannel.paymentMade)
    }

    @MainActor
    func testRunStabilityCheckAbortsWhenBooksActuallyChangeAfterDecision() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)

        let appState = AppState()
        appState.databaseService = dbService
        appState.priceService.setPriceForTesting(100_000.0)

        let channelId = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
        let userChannelId = "test-chan-changed"
        let counterparty = "020202020202020202020202020202020202020202020202020202020202020202"

        appState.stableChannel.channelId = channelId
        appState.stableChannel.userChannelId = userChannelId
        appState.stableChannel.counterparty = counterparty
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 100_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)
        appState.stableChannel.lastStabilityPayment = 0

        // In DB, save with backingSats = 0 (simulating concurrent rebalance / books changed)
        try dbService.channelRepo.saveChannel(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: 0.0,
            backingSats: 0,
            nativeSats: 100_000,
            note: "",
            receiverSats: 100_000,
            latestPrice: 100_000.0
        )

        let channel = makeMockChannel(
            channelId: channelId,
            userChannelId: userChannelId,
            outboundCapacityMsat: 75_000_000,
            unspendablePunishmentReserve: 25_000,
            channelValueSats: 200_000,
            isChannelReady: true
        )
        appState.nodeService.channelsOverride = [channel]
        appState.nodeService.lightningSyncAgeSecsOverride = 10

        var paymentAttempted = false
        appState.nodeService.sendStabilityPaymentOverride = { _, _, _ in
            paymentAttempted = true
            return "unexpected-payment-id"
        }

        // Execute runStabilityCheck
        appState.runStabilityCheck()

        // Since DB books had backingSats = 0, recheck evaluates to 0 msat != 75_000_000 msat,
        // so it safely clears the send slot and does NOT send payment.
        XCTAssertFalse(paymentAttempted)
        XCTAssertNil(dbService.stabilityRepo.loadPendingSend())
    }

    @MainActor
    func testRunStabilityCheckCapsSettlementToNextOutboundHtlcLimitWhenLowerThanCapacity() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)

        let appState = AppState()
        appState.databaseService = dbService
        appState.priceService.setPriceForTesting(100_000.0)

        let channelId = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
        let userChannelId = "test-chan-1"
        let counterparty = "020202020202020202020202020202020202020202020202020202020202020202"

        appState.stableChannel.channelId = channelId
        appState.stableChannel.userChannelId = userChannelId
        appState.stableChannel.counterparty = counterparty
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.expectedUSD = USD(amount: 0.0)
        appState.stableChannel.backingSats = 100_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 100_000)
        appState.stableChannel.lastStabilityPayment = 0
        appState.saveChannelToDB()

        // Outbound capacity is 75,000 sats (75,000,000 msat), but nextOutboundHtlcLimitMsat is 50,000 sats (50,000,000
        // msat)
        let channel = makeMockChannel(
            channelId: channelId,
            userChannelId: userChannelId,
            outboundCapacityMsat: 75_000_000,
            nextOutboundHtlcLimitMsat: 50_000_000,
            unspendablePunishmentReserve: 25_000,
            channelValueSats: 200_000,
            isChannelReady: true
        )
        appState.nodeService.channelsOverride = [channel]
        appState.nodeService.lightningSyncAgeSecsOverride = 10
        appState.nodeService.signMessageOverride = { _ in "mock-signature" }

        var paymentSentMsat: UInt64?
        appState.nodeService.sendStabilityPaymentOverride = { amountMsat, _, _ in
            paymentSentMsat = amountMsat
            return "mock-payment-id-htlc-limit"
        }

        appState.runStabilityCheck()

        // Settlement was capped to nextOutboundHtlcLimitMsat (50,000,000 msat)
        XCTAssertEqual(paymentSentMsat, 50_000_000)

        let recorded = dbService.paymentRepo.payment(paymentId: "mock-payment-id-htlc-limit")
        XCTAssertNotNil(recorded)
        XCTAssertEqual(recorded?.amountMsat, 50_000_000)
        XCTAssertEqual(recorded?.amountUSD, 50.0)

        let updatedChannel = try dbService.channelRepo.loadChannel(userChannelId: userChannelId)
        XCTAssertEqual(updatedChannel?.backingSats, 50_000)
        XCTAssertNil(dbService.stabilityRepo.loadPendingSend())
        XCTAssertTrue(appState.stableChannel.paymentMade)
    }

    @MainActor
    func testDetectOnchainDepositConsumesMarkerDuringChannelClosing() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let dbService = try DatabaseService(dataDir: tempDir)

        let appState = AppState()
        appState.databaseService = dbService
        appState.isChannelClosing = true

        // Insert pending channel_close operation with expected 100,000 sats
        _ = dbService.pendingOpRepo.insertPendingOperation(
            opId: "close-op-1",
            opType: "channel_close",
            fundingOutpointTxid: "chan-txid",
            fundingOutpointVout: 0,
            balanceSats: 100_000,
            balanceUsd: 100.0,
            btcPrice: 100_000.0,
            counterparty: "node-1"
        )

        appState.prevOnchainSats = 0
        appState.onchainBalanceSats = 99_000

        // Sweep arrives while channel is closing
        appState.detectOnchainDeposit()
        XCTAssertEqual(appState.prevOnchainSats, 99_000)

        // Marker must be consumed in database
        let consumed = try dbService.rawSQL.query(
            "SELECT 1 FROM consumed_close_sweeps WHERE payment_id = 'close-op-1'"
        )
        XCTAssertFalse(consumed.isEmpty)

        // Now close finishes and later close txid resolves
        appState.isChannelClosing = false
        dbService.pendingOpRepo.updatePendingOperation(
            opId: "close-op-1",
            closingTxid: "close-txid",
            status: "completed"
        )
        try dbService.paymentRepo.recordPayment(
            paymentId: "close-op-1",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: 100_000_000,
            amountUSD: 100.0,
            btcPrice: 100_000.0,
            counterparty: "node-1",
            status: "completed"
        )

        // An independent 100,000 sat deposit arrives later
        appState.onchainBalanceSats = 199_000
        appState.detectOnchainDeposit()

        // Because the close sweep was already consumed, this deposit is NOT swallowed as a sweep
        let depositPayments = (try? dbService.paymentRepo.getRecentPayments(limit: 10))?
            .filter { $0.paymentType == "onchain" && $0.direction == "received" } ?? []
        XCTAssertEqual(depositPayments.count, 1)
        XCTAssertEqual(depositPayments.first?.amountMsat, 100_000_000)
    }
}
