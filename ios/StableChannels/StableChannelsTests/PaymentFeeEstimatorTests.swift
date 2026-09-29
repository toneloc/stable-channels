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
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 10.0, isSendAll: false)
        XCTAssertEqual(fee, 10 * Constants.estimatedOnchainSendVBytes)
    }

    func testEstimateOnchainFee_sendAllUsesConstantsVBytes() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 10.0, isSendAll: true)
        XCTAssertEqual(fee, 10 * Constants.estimatedOnchainSendAllVBytes)
    }

    func testEstimateOnchainFee_customVBytes() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(
            feeRateSatVb: 12.0,
            isSendAll: false,
            sendVBytes: 250
        )
        XCTAssertEqual(fee, 3_000)
    }

    func testEstimateOnchainFee_fractionalRateRoundsUp() {
        // 1.12 sat/vB * 250 vB = 280 sat
        let fee = PaymentFeeEstimator.estimateOnchainFee(
            feeRateSatVb: 1.12,
            isSendAll: false,
            sendVBytes: 250
        )
        XCTAssertEqual(fee, 280)
    }

    func testEstimateOnchainFee_overflowSafety() {
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: Double.greatestFiniteMagnitude, isSendAll: false)
        XCTAssertEqual(fee, UInt64.max)
    }

    func testEstimateOnchainFee_nonFiniteOrNegativeRateReturnsZero() {
        XCTAssertEqual(PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: -5.0, isSendAll: false), 0)
        XCTAssertEqual(PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 0.0, isSendAll: false), 0)
        XCTAssertEqual(PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: Double.nan, isSendAll: false), 0)
        XCTAssertEqual(PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: Double.infinity, isSendAll: false), 0)
    }

    // MARK: - Network Fee Speed Tier Tests

    func testNetworkFeeSpeedTier_standardBaseline() {
        let base = 10.0
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base), 8.0, accuracy: 0.001)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), 10.0, accuracy: 0.001)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: base), 13.0, accuracy: 0.001)
    }

    func testNetworkFeeSpeedTier_lowFeeMempool() {
        let base = 1.0
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base), 0.8, accuracy: 0.001)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), 1.0, accuracy: 0.001)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: base), 1.3, accuracy: 0.001)
    }

    func testNetworkFeeSpeedTier_congestedMempool() {
        let base = 50.0
        XCTAssertEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base), 40.0, accuracy: 0.001)
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), 50.0, accuracy: 0.001)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: base), 65.0, accuracy: 0.001)
    }

    func testNetworkFeeSpeedTier_withRecommendedFees() {
        let rec = RecommendedFees(fastestFee: 25.0, halfHourFee: 18.0, hourFee: 12.0, minimumFee: 2.0)
        XCTAssertEqual(
            NetworkFeeSpeedTier.priority.effectiveRate(baseRate: 10.0, recommendedFees: rec),
            25.0,
            accuracy: 0.001
        )
        XCTAssertEqual(
            NetworkFeeSpeedTier.standard.effectiveRate(baseRate: 10.0, recommendedFees: rec),
            18.0,
            accuracy: 0.001
        )
        XCTAssertEqual(
            NetworkFeeSpeedTier.economy.effectiveRate(baseRate: 10.0, recommendedFees: rec),
            12.0,
            accuracy: 0.001
        )
    }

    func testNetworkFeeSpeedTier_monotonicityAcrossRange() {
        for rateInt in 1...200 {
            let rate = Double(rateInt)
            let eco = NetworkFeeSpeedTier.economy.effectiveRate(baseRate: rate)
            let std = NetworkFeeSpeedTier.standard.effectiveRate(baseRate: rate)
            let pri = NetworkFeeSpeedTier.priority.effectiveRate(baseRate: rate)

            XCTAssertGreaterThanOrEqual(eco, 0.1, "Economy must never drop below 0.1 sat/vB")
            XCTAssertLessThanOrEqual(eco, std, "Economy must not exceed standard")
            XCTAssertGreaterThan(pri, std, "Priority must strictly exceed standard")
        }
    }

    func testRecommendedFees_monotonicityClampedOnInvertedRates() {
        // Inverted rates: hourFee (20) > halfHour (10) > fastest (5)
        let inverted = RecommendedFees(fastestFee: 5.0, halfHourFee: 10.0, hourFee: 20.0, minimumFee: 2.0)
        let pri = inverted.rate(for: .priority)
        let std = inverted.rate(for: .standard)
        let eco = inverted.rate(for: .economy)

        XCTAssertGreaterThanOrEqual(pri, std, "Priority must clamp to at least standard")
        XCTAssertGreaterThanOrEqual(std, eco, "Standard must clamp to at least economy")
        XCTAssertGreaterThanOrEqual(eco, 2.0, "Economy must respect minimum fee")
    }

    func testRecommendedFees_quietMempoolAllOne() {
        let quiet = RecommendedFees(fastestFee: 1.0, halfHourFee: 1.0, hourFee: 1.0, economyFee: 1.0, minimumFee: 1.0)
        XCTAssertEqual(quiet.rate(for: .priority), 1.0, accuracy: 0.001)
        XCTAssertEqual(quiet.rate(for: .standard), 1.0, accuracy: 0.001)
        XCTAssertEqual(quiet.rate(for: .economy), 1.0, accuracy: 0.001)
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
