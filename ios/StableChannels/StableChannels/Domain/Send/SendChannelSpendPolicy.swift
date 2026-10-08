import Foundation
import LDKNode

/// Calculates spendable balances and channel routing fee parameters based on channel state.
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
            if !readyChannels.isEmpty && !isSweeping {
                let channelOutbound = readyChannels.map(\.outboundCapacityMsat).reduce(0, +) / 1000
                return min(channelOutbound, lightningBalanceSats)
            }
            return onchainBalanceSats
        }
    }

    /// Resolves primary channel routing fee parameters from active channels, falling back to
    /// safe system defaults when unconfigured or when channels are offline.
    static func forwardingFeeParameters(
        channels: [ChannelDetails]
    ) -> (baseMsat: UInt64, proportionalMillionths: UInt64) {
        let channel = channels.first(where: \.isChannelReady)
        let base = channel?.counterpartyForwardingInfoFeeBaseMsat.map { UInt64($0) }
            ?? UInt64(Constants.lightningDefaultForwardingFeeBaseMsat)
        let prop = channel?.counterpartyForwardingInfoFeeProportionalMillionths.map { UInt64($0) }
            ?? UInt64(Constants.lightningDefaultForwardingFeeProportionalMillionths)
        return (base, prop)
    }

    /// True if an on-chain destination will be dispatched as a splice-out from an open channel.
    static func isSpliceOut(channels: [ChannelDetails], isSweeping: Bool) -> Bool {
        channels.contains(where: \.isChannelReady) && !isSweeping
    }
}
