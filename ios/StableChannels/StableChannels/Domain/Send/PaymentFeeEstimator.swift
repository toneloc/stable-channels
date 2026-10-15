import Foundation

/// Pure calculations for Lightning routing and onchain network fee estimates.
enum PaymentFeeEstimator {
    /// Ceiling on total Lightning routing fees; mirrors LDK's default max_total_routing_fee_msat.
    static func maxLightningRoutingFeeMsat(amountMsat: UInt64) -> UInt64 {
        saturatingAdd(amountMsat / 100, 50_000)
    }

    /// The same ceiling in whole sats, rounded UP so the shown/reserved value always covers the enforced one.
    static func maxLightningRoutingFeeSats(sats: UInt64) -> UInt64 {
        maxLightningRoutingFeeSats(amountMsat: saturatingMultiply(sats, 1_000))
    }

    /// The ceiling for an exact msat amount (e.g. a fixed-amount invoice), rounded up to whole sats.
    static func maxLightningRoutingFeeSats(amountMsat: UInt64) -> UInt64 {
        guard amountMsat > 0 else { return 0 }
        return roundUpToSats(maxLightningRoutingFeeMsat(amountMsat: amountMsat))
    }

    /// Most an exact msat Lightning payment can debit (amount plus fee ceiling), rounded up to whole sats.
    static func maxLightningDebitSats(amountMsat: UInt64) -> UInt64 {
        guard amountMsat > 0 else { return 0 }
        return roundUpToSats(saturatingAdd(amountMsat, maxLightningRoutingFeeMsat(amountMsat: amountMsat)))
    }

    private static func roundUpToSats(_ msat: UInt64) -> UInt64 {
        msat / 1_000 + (msat % 1_000 == 0 ? 0 : 1)
    }

    /// Converts sat/vB to the sat/kwu LDK broadcasts with (1 vB = 4 WU), rounded up so a fractional
    /// rate such as 1.28 sat/vB is never sent below the rate shown and estimated.
    static func satPerKwu(fromSatPerVb feeRateSatVb: Double) -> UInt64 {
        UInt64((feeRateSatVb * 250).rounded(.up))
    }

    /// Estimates the expected onchain transaction fee based on fee rate and send type.
    static func estimateOnchainFee(
        feeRateSatVb: Double,
        isSendAll: Bool,
        sendVBytes: UInt64 = Constants.estimatedOnchainSendVBytes,
        sendAllVBytes: UInt64 = Constants.estimatedOnchainSendAllVBytes
    ) -> UInt64 {
        let vbytes = isSendAll ? sendAllVBytes : sendVBytes
        guard feeRateSatVb.isFinite && feeRateSatVb > 0 else { return 0 }
        let rawFee = Double(vbytes) * feeRateSatVb
        if rawFee >= Double(UInt64.max) { return UInt64.max }
        return UInt64(ceil(rawFee))
    }

    /// Estimates the expected onchain transaction fee for a splice-out operation.
    /// LDK constructs a 2-of-2 multisig spend transaction with new channel and destination outputs (~180 vB).
    static func estimateSpliceOutFee(
        feeRateSatVb: Double,
        vbytes: UInt64 = Constants.estimatedSpliceOutVBytes
    ) -> UInt64 {
        estimateOnchainFee(
            feeRateSatVb: feeRateSatVb,
            isSendAll: false,
            sendVBytes: vbytes
        )
    }

    // MARK: - Overflow-Safe Arithmetic

    static func saturatingMultiply(_ lhs: UInt64, _ rhs: UInt64) -> UInt64 {
        let result = lhs.multipliedReportingOverflow(by: rhs)
        return result.overflow ? UInt64.max : result.partialValue
    }

    static func saturatingAdd(_ lhs: UInt64, _ rhs: UInt64) -> UInt64 {
        let result = lhs.addingReportingOverflow(rhs)
        return result.overflow ? UInt64.max : result.partialValue
    }
}

/// Speed and confirmation target tiers for onchain Bitcoin network transactions.
enum NetworkFeeSpeedTier: String, CaseIterable, Identifiable, Sendable {
    case economy
    case standard
    case priority

    var id: String { rawValue }

    var title: String {
        switch self {
        case .economy: return String(localized: "tier_economy", defaultValue: "Economy")
        case .standard: return String(localized: "tier_standard", defaultValue: "Standard")
        case .priority: return String(localized: "tier_priority", defaultValue: "Priority")
        }
    }

    /// Estimated confirmation timeframe description.
    var estimatedTime: String {
        switch self {
        case .economy: return "> 1 hour"
        case .standard: return "≈ 30–60 min"
        case .priority: return "≈ 10–20 min"
        }
    }

    /// Target block depth in the blockchain.
    var targetBlocks: String {
        switch self {
        case .economy: return "12+ blocks"
        case .standard: return "3–6 blocks"
        case .priority: return "1–2 blocks"
        }
    }

    /// Computes the effective fee rate in satoshis per virtual byte (sat/vB).
    /// Uses live `recommendedFees` directly when available, or a bounded estimate.
    func effectiveRate(baseRate: Double, recommendedFees: RecommendedFees? = nil) -> Double {
        if let recommendedFees {
            return recommendedFees.rate(for: self)
        }
        let floor = Constants.minAllowedFeeRateSatVb
        let ceiling = Constants.maxAllowedFeeRateSatVb
        // Non-finite (NaN/infinite) inputs fall back to the floor; zero and negatives clamp up to it.
        let normalized = baseRate.isFinite ? min(ceiling, max(floor, baseRate)) : floor
        switch self {
        case .economy:
            return max(floor, normalized * 0.8)
        case .standard:
            return normalized
        case .priority:
            // The step keeps priority strictly above standard at the floor (1.3 vs 1.0).
            return min(ceiling, max(normalized + Self.priorityMinimumStepSatVb, normalized * 1.3))
        }
    }

    /// Minimum gap between priority and standard; a step, not a fee-rate floor.
    private static let priorityMinimumStepSatVb = 0.1
}
