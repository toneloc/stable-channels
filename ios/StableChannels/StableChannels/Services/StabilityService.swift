import Foundation
import LDKNode

/// Pure stability logic — direct port of src/stable.rs
enum StabilityService {
    // MARK: - Reconciliation

    /// Below this the position counts as zero-target; a zero target with no backing is closed.
    static let minimumStableUSD: Double = 0.01

    /// Reconcile an outgoing payment against the stable position.
    /// Returns the USD amount the stable target dropped by, or nil if fully covered by native BTC.
    static func reconcileOutgoing(_ sc: inout StableChannel, price: Double) -> Double? {
        guard sc.backingSats > 0, price > 0.0 else { return nil }

        let userSats = sc.stableReceiverBTC.sats
        guard sc.backingSats > userSats else { return nil }

        let overflowSats = sc.backingSats - userSats
        let usdOverflow = Double(overflowSats) / Double(Constants.satsInBTC) * price
        let oldExpected = sc.expectedUSD.amount
        let newExpected = max(oldExpected - usdOverflow, 0.0)

        sc.expectedUSD = USD(amount: newExpected)
        // Preserve sats, don't re-peg. The overflow is exactly what left the channel, so the
        // sats that remain are the backing. Re-pegging to newExpected/price left backing ABOVE
        // the live balance whenever the position was below par.
        // At the zero boundary the residue stays backing: a $0 target with sats still backing it
        // is an unsettled LSP surplus, which checkStabilityAction now settles as a normal
        // above-par PAY. Zeroing it here would release the LSP's sats to the user as native BTC.
        sc.backingSats = userSats
        recomputeNative(&sc)

        // Report only the target drop; past the zero boundary the rest of the overflow is surplus.
        return oldExpected - newExpected
    }

    struct RepairResult: Equatable {
        let overflowSats: UInt64
        let usdDeducted: Double
        let oldExpectedUSD: Double
        let newExpectedUSD: Double
    }

    /// Heal books that claim more backing than the channel holds.
    /// Backing > receiver cannot happen in normal operation: backing is a slice of that
    /// balance. It means a withdrawal moved sats out without its stable-books deduction.
    @discardableResult
    static func repairBooksAboveLiveBalance(_ sc: inout StableChannel, price: Double) -> RepairResult? {
        let receiverSats = sc.stableReceiverBTC.sats
        guard sc.backingSats > receiverSats, price > 0.0 else { return nil }

        let overflowSats = sc.backingSats - receiverSats
        let usdOverflow = Double(overflowSats) / Double(Constants.satsInBTC) * price
        let oldExpected = sc.expectedUSD.amount
        let newExpected = max(oldExpected - usdOverflow, 0.0)

        sc.expectedUSD = USD(amount: newExpected)
        // At the zero boundary the residue stays backing: a $0 target with sats still backing
        // it is an unsettled LSP surplus, which the stability machinery settles as a
        // normal above-par payment -- see reconcileOutgoing().
        sc.backingSats = receiverSats
        recomputeNative(&sc)

        // Report only the target drop; past the zero boundary the overflow is surplus.
        return RepairResult(
            overflowSats: overflowSats,
            usdDeducted: oldExpected - newExpected,
            oldExpectedUSD: oldExpected,
            newExpectedUSD: newExpected
        )
    }

    /// Reconcile a forwarded payment on the LSP side.
    /// `userSats` MUST be the balance BEFORE the spend — callers with a post-spend
    /// balance must add totalForwardedSats back first, or stable is over-deducted.
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

    /// Whether a spend of `amountSats` would consume sats owed to the LSP. Only
    /// the excess over the native (non-backing) balance AND the stable target itself touches
    /// the surplus: spending into backing first shrinks the target, which leaves the surplus
    /// owed to the LSP unchanged. A spend that exhausts the target eats the surplus directly.
    static func spendConsumesLspSurplus(_ sc: StableChannel, price: Double, amountSats: UInt64) -> Bool {
        guard checkStabilityAction(sc, price: price).action == .pay else { return false }
        let nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats : 0
        let overflowSats = Int64(amountSats) - Int64(nativeSats)
        guard overflowSats > 0 else { return false }
        let overflowUsd = Double(overflowSats) / Double(Constants.satsInBTC) * price
        return overflowUsd > sc.expectedUSD.amount
    }

    /// Recompute native BTC from receiver sats and backing sats.
    static func recomputeNative(_ sc: inout StableChannel) {
        let nativeSats = sc.stableReceiverBTC.sats >= sc.backingSats
            ? sc.stableReceiverBTC.sats - sc.backingSats
            : 0
        sc.nativeChannelBTC = Bitcoin(sats: nativeSats)
        sc.nativeSats = nativeSats
    }

    /// Reconcile an incoming payment — backingSats stays the same, native absorbs the increase.
    static func reconcileIncoming(_ sc: inout StableChannel) {
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

        // No backing means no stable position -- nothing to drift.
        // A sub-cent target with no backing is a closed position. A sub-cent target WITH
        // backing is not: those sats are an unsettled LSP surplus and must settle
        // like any other above-par balance instead of being stranded by this bail.
        guard sc.backingSats > 0 else {
            return StabilityCheckResult(
                action: .stable,
                percentFromPar: 0.0,
                stableUSDValue: 0.0,
                targetUSD: targetUSD,
                dollarsFromPar: 0.0
            )
        }

        let stableUSDValue = Double(sc.backingSats) / 100_000_000.0 * price

        let dollarsFromPar = stableUSDValue - targetUSD
        // Clamp the denominator: at a zero/tiny target an unclamped ratio is 0 (or explodes),
        // which would pin percentFromPar inside the deadband and block settlement of the residue.
        let percentFromPar = abs(dollarsFromPar / max(targetUSD, minimumStableUSD)) * 100.0
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
        // Always keep channelId & counterparty current
        sc.channelId = channel.channelId
        sc.counterparty = channel.counterpartyNodeId

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

    // MARK: - Settlement Calculation

    /// Calculate the stability settlement amount in millisatoshis, capped to spendable channel capacity
    /// and single-HTLC limits. Pure functional core arithmetic.
    static func calculateSettlementAmountMsat(
        dollarsFromPar: Double,
        price: Double,
        outboundCapacityMsat: UInt64? = nil,
        nextOutboundHtlcLimitMsat: UInt64? = nil
    ) -> UInt64 {
        guard price > 0, dollarsFromPar != 0 else { return 0 }
        let uncappedMsat = USD(amount: abs(dollarsFromPar)).toMsats(price: price) / 1000 * 1000
        guard let outboundCapacityMsat else { return uncappedMsat }

        let effectiveCapacity: UInt64
        if let nextOutboundHtlcLimitMsat {
            effectiveCapacity = min(outboundCapacityMsat, nextOutboundHtlcLimitMsat)
        } else {
            effectiveCapacity = outboundCapacityMsat
        }
        let maxSpendableMsat = effectiveCapacity / 1000 * 1000
        return min(uncappedMsat, maxSpendableMsat)
    }
}
