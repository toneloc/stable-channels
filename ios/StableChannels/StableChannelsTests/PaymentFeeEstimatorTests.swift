import XCTest
@testable import StableChannels

final class PaymentFeeEstimatorTests: XCTestCase {
    // MARK: - Lightning Routing Fee Tests

    func testEstimateLightningFee_zeroSatsReturnsZero() {
        let fee = PaymentFeeEstimator.estimateLightningFee(
            sats: 0,
            baseMsat: 1_000,
            proportionalMillionths: 500
        )
        XCTAssertEqual(fee, 0)
    }

    func testEstimateLightningFee_standardForwardingCalculation() {
        // 10,000 sats = 10,000,000 msat
        // proportional: 10,000,000 * 500 / 1,000,000 = 5,000 msat
        // total: 1,000 base + 5,000 prop = 6,000 msat = 6 sats
        let fee = PaymentFeeEstimator.estimateLightningFee(
            sats: 10_000,
            baseMsat: 1_000,
            proportionalMillionths: 500
        )
        XCTAssertEqual(fee, 6)
    }

    func testEstimateLightningFee_ceilingDivisionRoundsUp() {
        // 1 sat = 1,000 msat. 1 msat base, 0 ppm -> 1 msat -> rounds up to 1 sat
        let fee = PaymentFeeEstimator.estimateLightningFee(
            sats: 1,
            baseMsat: 1,
            proportionalMillionths: 0
        )
        XCTAssertEqual(fee, 1)
    }

    func testEstimateLightningFee_overflowSafetyDoesNotCrash() {
        let fee = PaymentFeeEstimator.estimateLightningFee(
            sats: UInt64.max,
            baseMsat: 1_000,
            proportionalMillionths: 5_000
        )
        let expected = (1_999 + (UInt64.max / 1_000_000)) / 1_000
        XCTAssertEqual(fee, expected)
        XCTAssertGreaterThan(fee, 18_000_000_000)
    }

    // MARK: - Onchain Fee Estimation Tests

    func testEstimateOnchainFee_standardSendUsesEstimatedVBytes() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 10, isSendAll: false)
        XCTAssertEqual(fee, 10 * Constants.estimatedOnchainSendVBytes)
    }

    func testEstimateOnchainFee_sendAllUsesConstantsVBytes() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 10, isSendAll: true)
        XCTAssertEqual(fee, 10 * Constants.estimatedOnchainSendAllVBytes)
    }

    func testEstimateOnchainFee_customVBytes() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(
            feeRateSatVb: 12,
            isSendAll: false,
            sendVBytes: 250
        )
        XCTAssertEqual(fee, 3_000)
    }

    func testEstimateOnchainFee_overflowSafety() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: UInt64.max, isSendAll: false)
        XCTAssertEqual(fee, UInt64.max)
    }

    // MARK: - Network Fee Speed Tier Tests

    func testNetworkFeeSpeedTier_standardBaseline() {
        let base: UInt64 = 10
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base), 8)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), 10)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: base), 13)
    }

    func testNetworkFeeSpeedTier_lowFeeMempool() {
        let base: UInt64 = 1
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base), 1)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), 1)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: base), 2)
    }

    func testNetworkFeeSpeedTier_congestedMempool() {
        let base: UInt64 = 50
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base), 40)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), 50)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: base), 65)
    }

    func testNetworkFeeSpeedTier_withRecommendedFees() {
        let rec = RecommendedFees(fastestFee: 25, halfHourFee: 18, hourFee: 12, minimumFee: 2)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: 10, recommendedFees: rec), 25)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: 10, recommendedFees: rec), 18)
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: 10, recommendedFees: rec), 12)
    }

    func testNetworkFeeSpeedTier_monotonicityAcrossRange() {
        for rate: UInt64 in 1...200 {
            let eco = NetworkFeeSpeedTier.economy.effectiveRate(baseRate: rate)
            let std = NetworkFeeSpeedTier.standard.effectiveRate(baseRate: rate)
            let pri = NetworkFeeSpeedTier.priority.effectiveRate(baseRate: rate)

            XCTAssertGreaterThanOrEqual(eco, 1, "Economy must never drop below 1 sat/vB")
            XCTAssertLessThanOrEqual(eco, std, "Economy must not exceed standard")
            XCTAssertGreaterThan(pri, std, "Priority must strictly exceed standard")
        }
    }

    func testRecommendedFees_monotonicityClampedOnInvertedRates() {
        // Inverted rates: hourFee (20) > halfHour (10) > fastest (5)
        let inverted = RecommendedFees(fastestFee: 5, halfHourFee: 10, hourFee: 20, minimumFee: 2)
        let pri = inverted.rate(for: .priority)
        let std = inverted.rate(for: .standard)
        let eco = inverted.rate(for: .economy)

        XCTAssertGreaterThanOrEqual(pri, std, "Priority must clamp to at least standard")
        XCTAssertGreaterThanOrEqual(std, eco, "Standard must clamp to at least economy")
        XCTAssertGreaterThanOrEqual(eco, 2, "Economy must respect minimum fee")
    }

    func testRecommendedFees_quietMempoolAllOne() {
        let quiet = RecommendedFees(fastestFee: 1, halfHourFee: 1, hourFee: 1, economyFee: 1, minimumFee: 1)
        XCTAssertEqual(quiet.rate(for: .priority), 1)
        XCTAssertEqual(quiet.rate(for: .standard), 1)
        XCTAssertEqual(quiet.rate(for: .economy), 1)
    }

    func testNetworkFeeSpeedTier_metadataFields() {
        for tier in NetworkFeeSpeedTier.allCases {
            XCTAssertFalse(tier.title.isEmpty)
            XCTAssertFalse(tier.estimatedTime.isEmpty)
            XCTAssertFalse(tier.targetBlocks.isEmpty)
            XCTAssertEqual(tier.id, tier.rawValue)
        }
    }
}
