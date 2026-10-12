import Foundation
import LDKNode

/// Calculates spendable balances and splice routing based on channel state.
enum SendChannelSpendPolicy {
    /// Computes the spendable satoshis available for a given destination, taking into account
    /// channel outbound capacity for Lightning routing and splice-out constraints for on-chain.
    static func availableSpendableSats(
        destination: SendDestination?,
        channels: [ChannelDetails],
        lightningBalanceSats: UInt64,
        onchainBalanceSats: UInt64,
        totalBalanceSats: UInt64,
        isSweeping: Bool
    ) -> UInt64 {
        guard let destination else { return totalBalanceSats }
        let readyChannels = channels.filter(\.isChannelReady)
        switch destination {
        case .bolt11, .bolt12, .lightningAddress, .lnurlPay:
            if !readyChannels.isEmpty {
                let channelOutbound = readyChannels.map(\.outboundCapacityMsat).reduce(0, +) / 1000
                return min(channelOutbound, lightningBalanceSats)
            }
            return lightningBalanceSats
        case .onchain:
            return availableOnchainSpendableSats(
                channels: channels,
                lightningBalanceSats: lightningBalanceSats,
                onchainBalanceSats: onchainBalanceSats,
                isSweeping: isSweeping
            )
        }
    }

    /// What an on-chain send can spend. A splice-out is executed against a single specific channel,
    /// so outbound capacity cannot be aggregated across channels: it is bounded by the largest ready
    /// channel and the Lightning balance. Without a ready channel, or during a sweep, the on-chain
    /// wallet pays.
    static func availableOnchainSpendableSats(
        channels: [ChannelDetails],
        lightningBalanceSats: UInt64,
        onchainBalanceSats: UInt64,
        isSweeping: Bool
    ) -> UInt64 {
        let readyChannels = channels.filter(\.isChannelReady)
        guard !readyChannels.isEmpty, !isSweeping else { return onchainBalanceSats }
        let maxSingleChannelOutbound = readyChannels.map { $0.outboundCapacityMsat / 1000 }.max() ?? 0
        return min(maxSingleChannelOutbound, lightningBalanceSats)
    }

    /// Selects the best ready channel capable of funding an on-chain splice-out of `requiredSats` (including fee).
    /// Chooses the smallest channel that has sufficient outbound capacity (best fit), or falls back to the largest.
    static func selectSpliceChannel(
        channels: [ChannelDetails],
        requiredSats: UInt64
    ) -> ChannelDetails? {
        let ready = channels.filter(\.isChannelReady)
        let sufficient = ready.filter { ($0.outboundCapacityMsat / 1000) >= requiredSats }
        if let bestFit = sufficient.min(by: { $0.outboundCapacityMsat < $1.outboundCapacityMsat }) {
            return bestFit
        }
        return ready.max(by: { $0.outboundCapacityMsat < $1.outboundCapacityMsat })
    }

    /// True if an on-chain destination will be dispatched as a splice-out from an open channel.
    static func isSpliceOut(channels: [ChannelDetails], isSweeping: Bool) -> Bool {
        channels.contains(where: \.isChannelReady) && !isSweeping
    }

    /// A Max send to an on-chain address with no splice-out empties the on-chain wallet in one
    /// transaction whose fee is paid from the balance. A splice-out Max draws on one channel instead.
    static func isOnchainSendAll(
        isMax: Bool,
        destination: SendDestination?,
        channels: [ChannelDetails],
        isSweeping: Bool
    ) -> Bool {
        guard isMax, case .onchain = destination else { return false }
        return !isSpliceOut(channels: channels, isSweeping: isSweeping)
    }

    /// Computes the largest sendable amount such that amount + fee(amount) fits inside the available balance.
    /// Runs a monotonic contraction mapping fixed-point iteration to handle non-linear/proportional routing fees.
    static func calculateMaxSendable(
        available: UInt64,
        feeEstimator: (UInt64) -> UInt64
    ) -> UInt64 {
        guard available > 0 else { return 0 }

        func fits(_ amount: UInt64) -> Bool {
            let fee = feeEstimator(amount)
            return fee <= available && amount <= available - fee
        }

        var candidate = available
        for _ in 0..<8 {
            let fee = feeEstimator(candidate)
            let next = available > fee ? (available - fee) : 0
            if next == candidate { break }
            candidate = next
        }
        var steps = 0
        while candidate > 0 && !fits(candidate) && steps < 64 {
            candidate -= 1
            steps += 1
        }
        return fits(candidate) ? candidate : 0
    }
}
