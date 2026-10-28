import Foundation
import LDKNode

/// Funds from a closed channel that are not in the onchain wallet yet (#358). After a force close
/// they are timelocked on chain, then LDK sweeps them to the wallet. Open channels are left out. A
/// coop close pays straight to the wallet, so it is left out whether or not it has confirmed. A
/// sweep is counted until LDK broadcasts it: the wallet counts it from its next sync.
struct ClosedChannelFunds: Equatable {
    let amountSats: UInt64
    let blocksLeft: UInt32?
    /// The part in sweeps LDK has not broadcast. The rest is in LDK's lightning balance.
    var sweepSats: UInt64 = 0

    /// Only the sweeps LDK has not broadcast, for a total that counts LDK's lightning balance.
    var sweepsOnly: ClosedChannelFunds? {
        sweepSats > 0 ? ClosedChannelFunds(amountSats: sweepSats, blocksLeft: nil, sweepSats: sweepSats) : nil
    }

    /// Without the sweeps LDK has not broadcast, for a total that counts them.
    var withoutSweeps: ClosedChannelFunds? {
        amountSats > sweepSats ? ClosedChannelFunds(amountSats: amountSats - sweepSats, blocksLeft: blocksLeft) : nil
    }

    /// Sweeps LDK has not broadcast yet. A broadcast sweep is left out, as the wallet counts it once it
    /// sees the tx. LDK re-signs an unconfirmed sweep each block to a new wallet address, so its latest
    /// txid is often not the one the mempool and the wallet have.
    static func unbroadcastSweepSats(_ pendingSweeps: [PendingSweepBalance]) -> UInt64 {
        pendingSweeps.reduce(0) { sum, sweep in
            if case let .pendingBroadcast(_, amount) = sweep { return sum + amount }
            return sum
        }
    }

    static func isCooperative(_ reason: ClosureReason?) -> Bool {
        switch reason {
        case .legacyCooperativeClosure, .counterpartyInitiatedCooperativeClosure,
             .locallyInitiatedCooperativeClosure:
            true
        default:
            false
        }
    }

    static func from(
        lightningBalances: [LightningBalance],
        pendingSweeps: [PendingSweepBalance],
        openChannelIds: Set<String>,
        coopClosedChannelIds: Set<String>,
        currentHeight: UInt32
    ) -> ClosedChannelFunds? {
        var amountSats: UInt64 = 0
        var unlockHeight: UInt32?
        for balance in lightningBalances {
            switch balance {
            // The wallet shows a coop close as soon as it sees the close tx.
            case let .claimableOnChannelClose(channelId, _, amount, _, _, _, _, _):
                if openChannelIds.contains(channelId) || coopClosedChannelIds.contains(channelId) { continue }
                amountSats += amount
            case let .claimableAwaitingConfirmations(_, _, amount, confirmationHeight, source):
                if source == .coopClose { continue }
                amountSats += amount
                unlockHeight = max(unlockHeight ?? confirmationHeight, confirmationHeight)
            case let .contentiousClaimable(_, _, amount, _, _, _):
                amountSats += amount
            // Counted as LDK's claimable_amount_satoshis counts them: an HTLC of our own payment, or
            // one waiting for a preimage, may never come back to us. An open channel lists its
            // pending HTLCs too.
            case let .maybeTimeoutClaimableHtlc(channelId, _, amount, _, _, outboundPayment):
                if outboundPayment || openChannelIds.contains(channelId) { continue }
                amountSats += amount
            case .maybePreimageClaimableHtlc:
                break
            case let .counterpartyRevokedOutputClaimable(_, _, amount):
                amountSats += amount
            }
        }
        let sweepSats = unbroadcastSweepSats(pendingSweeps)
        amountSats += sweepSats
        guard amountSats > 0 else { return nil }
        return ClosedChannelFunds(
            amountSats: amountSats,
            blocksLeft: unlockHeight.map { $0 > currentHeight ? $0 - currentHeight : 0 },
            sweepSats: sweepSats
        )
    }

    /// "1 block (about 10 minutes)", "288 blocks (about 2 days)", rounding up.
    static func blocksText(_ blocks: UInt32) -> String {
        let minutes = blocks * 10
        let about = if minutes < 60 {
            "\(minutes) minutes"
        } else if minutes < 24 * 60 {
            count((minutes + 59) / 60, "hour")
        } else {
            count((minutes + 24 * 60 - 1) / (24 * 60), "day")
        }
        return "\(count(blocks, "block")) (about \(about))"
    }

    private static func count(_ n: UInt32, _ unit: String) -> String {
        n == 1 ? "1 \(unit)" : "\(n) \(unit)s"
    }

    func message(btcPrice: Double) -> String {
        let amount = btcPrice > 0
            ? (Double(amountSats) / Double(Constants.satsInBTC) * btcPrice).usdFormatted
            : "\(amountSats.btcSpacedFormatted) BTC"
        let timing = if let blocksLeft, blocksLeft > 0 {
            "It unlocks in \(Self.blocksText(blocksLeft)), then moves to your onchain wallet automatically."
        } else {
            "It moves to your onchain wallet automatically, usually within a few hours."
        }
        return "\(amount) is safe on the Bitcoin blockchain. \(timing) Keep the app installed until it arrives."
    }
}
