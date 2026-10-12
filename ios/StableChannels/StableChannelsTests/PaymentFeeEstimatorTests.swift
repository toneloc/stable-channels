import XCTest
@testable import StableChannels

final class PaymentFeeEstimatorTests: XCTestCase {
    // MARK: - Lightning Routing Fee Ceiling Tests

    func testMaxLightningRoutingFeeMsat_onePercentPlusFiftySats() {
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeMsat(amountMsat: 100_000_000), 1_050_000)
    }

    func testMaxLightningRoutingFeeMsat_zeroAmountIsBaseOnly() {
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeMsat(amountMsat: 0), 50_000)
    }

    func testMaxLightningRoutingFeeMsat_maxAmountDoesNotTrap() {
        // UInt64.max / 100 + 50_000 fits without saturating.
        XCTAssertEqual(
            PaymentFeeEstimator.maxLightningRoutingFeeMsat(amountMsat: UInt64.max),
            184_467_440_737_145_516
        )
    }

    func testMaxLightningRoutingFeeSats_zeroSatsReturnsZero() {
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(sats: 0), 0)
    }

    func testMaxLightningRoutingFeeSats_wholeSatCeiling() {
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(sats: 100_000), 1_050)
    }

    func testMaxLightningRoutingFeeSats_roundsUpPartialSat() {
        // 1_000 msat / 100 + 50_000 = 50_010 msat -> 51 sats
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(sats: 1), 51)
    }

    func testMaxLightningRoutingFeeSats_maxSatsSaturatesWithoutTrap() {
        // sats * 1_000 saturates to UInt64.max msat; cap = 184_467_440_737_145_516 msat -> rounded up.
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(sats: UInt64.max), 184_467_440_737_146)
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

    func testEstimateSpliceOutFee_usesSpliceConstantsVBytes() {
        let fee = PaymentFeeEstimator.estimateSpliceOutFee(feeRateSatVb: 10.0)
        XCTAssertEqual(fee, 10 * Constants.estimatedSpliceOutVBytes)
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
        XCTAssertEqual(
            NetworkFeeSpeedTier.economy.effectiveRate(baseRate: base),
            Constants.minAllowedFeeRateSatVb,
            accuracy: 0.001
        )
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

            XCTAssertGreaterThanOrEqual(
                eco,
                Constants.minAllowedFeeRateSatVb,
                "Economy must never drop below the node floor"
            )
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

    func testSpliceOutVByteEstimateIsPinned() {
        // Pinned to a literal: other tests derive their expectation from this constant.
        XCTAssertEqual(Constants.estimatedSpliceOutVBytes, 180)
    }

    func testMaxLightningRoutingFeeSats_exactMsatRoundsUpSubSatRemainder() {
        // 100,000.999 sats: cap is 1,050,009 msat, which must show as 1,051 sats, not 1,050.
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(amountMsat: 100_000_999), 1051)
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(amountMsat: 100_000_000), 1050)
        XCTAssertEqual(PaymentFeeEstimator.maxLightningRoutingFeeSats(amountMsat: 0), 0)
    }

    func testMaxLightningDebitSats_coversAmountPlusCeilingRoundedUp() {
        // 100,000,999 + 1,050,009 = 101,051,008 msat -> 101,052 sats.
        XCTAssertEqual(PaymentFeeEstimator.maxLightningDebitSats(amountMsat: 100_000_999), 101_052)
        XCTAssertEqual(PaymentFeeEstimator.maxLightningDebitSats(amountMsat: 100_000_000), 101_050)
        XCTAssertEqual(PaymentFeeEstimator.maxLightningDebitSats(amountMsat: 0), 0)
        XCTAssertEqual(PaymentFeeEstimator.maxLightningDebitSats(amountMsat: UInt64.max), UInt64.max / 1000 + 1)
    }

    // MARK: - Shared Fee-Rate Bounds

    func testRecommendedFees_subFloorRatesClampToSharedFloor() {
        let floor = Constants.minAllowedFeeRateSatVb
        let subFloor = RecommendedFees(
            fastestFee: 0.5,
            halfHourFee: 0.5,
            hourFee: 0.5,
            economyFee: 0.5,
            minimumFee: 0.5
        )
        for tier in NetworkFeeSpeedTier.allCases {
            XCTAssertGreaterThanOrEqual(subFloor.rate(for: tier), floor, "\(tier) must not drop below the node floor")
            XCTAssertGreaterThanOrEqual(tier.effectiveRate(baseRate: 0.5, recommendedFees: subFloor), floor)
        }
    }

    func testNetworkFeeSpeedTier_subFloorBaseRateWithoutRecommendedFees() {
        let floor = Constants.minAllowedFeeRateSatVb
        for tier in NetworkFeeSpeedTier.allCases {
            XCTAssertGreaterThanOrEqual(
                tier.effectiveRate(baseRate: 0.3),
                floor,
                "\(tier) must not drop below the node floor"
            )
        }
        XCTAssertGreaterThan(
            NetworkFeeSpeedTier.priority.effectiveRate(baseRate: 0.3),
            NetworkFeeSpeedTier.standard.effectiveRate(baseRate: 0.3),
            "Priority must strictly exceed standard at the floor"
        )
    }

    func testNetworkFeeSpeedTier_zeroNegativeAndNaNBaseRateStayWithinBounds() {
        let floor = Constants.minAllowedFeeRateSatVb
        let ceiling = Constants.maxAllowedFeeRateSatVb
        for base in [0.0, -5.0, Double.nan] {
            XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: base), floor)
            for tier in NetworkFeeSpeedTier.allCases {
                let rate = tier.effectiveRate(baseRate: base)
                XCTAssertGreaterThanOrEqual(rate, floor, "\(tier) at base \(base)")
                XCTAssertLessThanOrEqual(rate, ceiling, "\(tier) at base \(base)")
            }
        }
    }

    func testNetworkFeeSpeedTier_aboveCeilingClampsToCeiling() {
        let ceiling = Constants.maxAllowedFeeRateSatVb
        let huge = RecommendedFees(
            fastestFee: 20_000.0,
            halfHourFee: 20_000.0,
            hourFee: 20_000.0,
            economyFee: 20_000.0,
            minimumFee: 20_000.0
        )
        for tier in NetworkFeeSpeedTier.allCases {
            XCTAssertEqual(tier.effectiveRate(baseRate: 20_000.0, recommendedFees: huge), ceiling, "\(tier)")
        }
        // Without recommended fees, standard and priority hit the ceiling; economy stays below it.
        XCTAssertEqual(NetworkFeeSpeedTier.standard.effectiveRate(baseRate: 20_000.0), ceiling)
        XCTAssertEqual(NetworkFeeSpeedTier.priority.effectiveRate(baseRate: 20_000.0), ceiling)
        XCTAssertLessThanOrEqual(NetworkFeeSpeedTier.economy.effectiveRate(baseRate: 20_000.0), ceiling)
    }
}
