import Foundation
import LDKNode

/// Pure stability logic — direct port of src/stable.rs
enum StabilityService {

    // MARK: - Reconciliation

    /// Reconcile an outgoing payment against the stable position.
    /// Returns the USD amount deducted from stable, or nil if fully covered by native BTC.
    static func reconcileOutgoing(_ sc: inout StableChannel, price: Double) -> Double? {
        guard sc.expectedUSD.amount > 0.01, sc.backingSats > 0, price > 0.0 else { return nil }

        let userSats = sc.stableReceiverBTC.sats
        guard sc.backingSats > userSats else { return nil }

        let overflowSats = sc.backingSats - userSats
        let usdToDeduct = Double(overflowSats) / Double(Constants.satsInBTC) * price
        let newExpected = max(sc.expectedUSD.amount - usdToDeduct, 0.0)

        sc.expectedUSD = USD(amount: newExpected)
        let btcAmount = newExpected / price
        sc.backingSats = UInt64(btcAmount * 100_000_000.0)
        sc.nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats : 0
        recomputeNative(&sc)

        return usdToDeduct
    }

    /// Reconcile a forwarded payment on the LSP side.
    /// Returns the USD amount deducted from stable, or nil if fully covered by native.
    static func reconcileForwarded(
        _ sc: inout StableChannel,
        userSats: UInt64,
        totalForwardedSats: UInt64,
        price: Double
    ) -> Double? {
        guard sc.expectedUSD.amount > 0.0, price > 0.0 else { return nil }

        let nativeSats = userSats >= sc.backingSats ? userSats - sc.backingSats : 0
        let overflowSats = totalForwardedSats >= nativeSats ? totalForwardedSats - nativeSats : 0

        guard overflowSats > 0 else { return nil }

        let usdToDeduct = Double(overflowSats) / Double(Constants.satsInBTC) * price
        let newExpected = max(sc.expectedUSD.amount - usdToDeduct, 0.0)

        sc.expectedUSD = USD(amount: newExpected)
        if price > 0.0 {
            let btcAmount = newExpected / price
            sc.backingSats = UInt64(btcAmount * 100_000_000.0)
        }
        sc.nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats : 0
        recomputeNative(&sc)

        return usdToDeduct
    }

    /// Pre-deduct stable balance for a known outgoing amount (e.g. splice-out).
    /// Returns the USD amount deducted, or nil if fully covered by native.
    static func deductOutgoing(_ sc: inout StableChannel, amountSats: UInt64, price: Double) -> Double? {
        guard sc.expectedUSD.amount > 0.01, price > 0.0 else { return nil }

        let nativeSats = sc.nativeChannelBTC.sats
        guard amountSats > nativeSats else { return nil }

        let overflowSats = amountSats - nativeSats
        let usdToDeduct = Double(overflowSats) / Double(Constants.satsInBTC) * price
        let newExpected = max(sc.expectedUSD.amount - usdToDeduct, 0.0)

        sc.expectedUSD = USD(amount: newExpected)
        let btcAmount = newExpected / price
        sc.backingSats = UInt64(btcAmount * 100_000_000.0)
        sc.nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats : 0
        recomputeNative(&sc)

        return usdToDeduct
    }

    /// Recompute native BTC from receiver sats and backing sats.
    static func recomputeNative(_ sc: inout StableChannel) {
        let nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats
            : 0
        sc.nativeChannelBTC = Bitcoin(sats: nativeSats)
    }

    /// Reconcile an incoming payment — derive backingSats from channel balance.
    static func reconcileIncoming(_ sc: inout StableChannel) {
        if sc.expectedUSD.amount > 0.0 && sc.latestPrice > 0.0 {
            let btcAmount = sc.expectedUSD.amount / sc.latestPrice
            sc.backingSats = UInt64(btcAmount * 100_000_000.0)
        }
        recomputeNative(&sc)
    }

    /// Apply a trade — set new expected USD and recalculate backing sats + native sats.
    static func applyTrade(_ sc: inout StableChannel, newExpectedUSD: Double, price: Double) {
        sc.expectedUSD = USD(amount: newExpectedUSD)
        if price > 0.0 {
            let btcAmount = newExpectedUSD / price
            sc.backingSats = UInt64(btcAmount * 100_000_000.0)
        }
        // native_sats is everything NOT backing the stable position
        sc.nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats : 0
        recomputeNative(&sc)
    }

    // MARK: - Stability Check

    enum StabilityAction: String {
        case stable = "STABLE"
        case highRiskNoAction = "HIGH_RISK_NO_ACTION"
        case checkOnly = "CHECK_ONLY"
        case pay = "PAY"
    }

    struct StabilityCheckResult {
        let action: StabilityAction
        let percentFromPar: Double
        let stableUSDValue: Double
        let targetUSD: Double
        let dollarsFromPar: Double
    }

    /// Determine the stability action without sending payment.
    static func checkStabilityAction(_ sc: StableChannel, price: Double) -> StabilityCheckResult {
        let targetUSD = sc.expectedUSD.amount

        let stableUSDValue: Double
        if sc.backingSats > 0 {
            stableUSDValue = Double(sc.backingSats) / 100_000_000.0 * price
        } else {
            stableUSDValue = sc.stableReceiverUSD.amount
        }

        let dollarsFromPar = stableUSDValue - targetUSD
        let percentFromPar = targetUSD > 0.0 ? abs(dollarsFromPar / targetUSD) * 100.0 : 0.0
        let isReceiverBelowExpected = stableUSDValue < targetUSD

        let action: StabilityAction
        if percentFromPar < Constants.stabilityThresholdPercent
            || abs(dollarsFromPar) < Constants.stabilityThresholdUSD {
            action = .stable
        } else if sc.riskLevel > Constants.maxRiskLevel {
            action = .highRiskNoAction
        } else if (sc.isStableReceiver && isReceiverBelowExpected)
                    || (!sc.isStableReceiver && !isReceiverBelowExpected) {
            action = .checkOnly
        } else {
            action = .pay
        }

        return StabilityCheckResult(
            action: action,
            percentFromPar: percentFromPar,
            stableUSDValue: stableUSDValue,
            targetUSD: targetUSD,
            dollarsFromPar: dollarsFromPar
        )
    }

    // MARK: - Balance Update

    /// Update balances on a StableChannel from LDK channel data.
    /// Returns true if a matching channel was found.
    @discardableResult
    static func updateBalances(
        _ sc: inout StableChannel,
        channels: [ChannelDetails],
        onchainBalanceSats: UInt64,
        price: Double
    ) -> Bool {
        if price > 0.0 {
            sc.latestPrice = price
        }

        // Update on-chain
        sc.onchainBTC = Bitcoin(sats: onchainBalanceSats)
        sc.onchainUSD = USD.fromBitcoin(sc.onchainBTC, price: sc.latestPrice)

        // Find matching channel
        let matchingChannel: ChannelDetails?
        if sc.userChannelId.isEmpty {
            matchingChannel = channels.first
        } else {
            matchingChannel = channels.first { $0.userChannelId == sc.userChannelId }
        }

        guard let channel = matchingChannel else { return false }

        // Auto-assign channel IDs if not set
        if sc.userChannelId.isEmpty {
            sc.userChannelId = channel.userChannelId
            sc.channelId = channel.channelId
        }
        // Always keep channelId current (it changes on splice)
        sc.channelId = channel.channelId

        // Skip balance update if channel is not ready yet — during ChannelPending,
        // outbound_capacity_msat is 0, which produces a misleading near-zero balance.
        guard channel.isChannelReady else { return true }

        let unspendablePunishmentSats = channel.unspendablePunishmentReserve ?? 0
        let ourBalanceSats = (channel.outboundCapacityMsat / 1000) + unspendablePunishmentSats
        let theirBalanceSats = channel.channelValueSats > ourBalanceSats
            ? channel.channelValueSats - ourBalanceSats : 0

        if sc.isStableReceiver {
            sc.stableReceiverBTC = Bitcoin(sats: ourBalanceSats)
            sc.stableProviderBTC = Bitcoin(sats: theirBalanceSats)
        } else {
            sc.stableProviderBTC = Bitcoin(sats: ourBalanceSats)
            sc.stableReceiverBTC = Bitcoin(sats: theirBalanceSats)
        }

        sc.stableReceiverUSD = USD.fromBitcoin(sc.stableReceiverBTC, price: sc.latestPrice)
        sc.stableProviderUSD = USD.fromBitcoin(sc.stableProviderBTC, price: sc.latestPrice)

        // Native BTC is the portion not backing the stable position
        recomputeNative(&sc)

        return true
    }
}
