import Foundation
import LDKNode

/// Funds from a closed channel that are not in the onchain wallet yet (#358). After a force close
/// they are timelocked on chain, then LDK sweeps them to the wallet. A closed channel is reported as
/// claimableOnChannelClose until its close tx confirms, so that is counted only for channels no
/// longer open. Confirmed coop close funds are left out: they are already in the wallet. A sweep
/// stays counted until it confirms, as the wallet may not show it before then.
struct ClosedChannelFunds: Equatable {
    let amountSats: UInt64
    let blocksLeft: UInt32?

    static func from(
        lightningBalances: [LightningBalance],
        pendingSweeps: [PendingSweepBalance],
        openChannelIds: Set<String>,
        currentHeight: UInt32
    ) -> ClosedChannelFunds? {
        var amountSats: UInt64 = 0
        var unlockHeight: UInt32?
        for balance in lightningBalances {
            switch balance {
            case let .claimableOnChannelClose(channelId, _, amount, _, _, _, _, _):
                if !openChannelIds.contains(channelId) { amountSats += amount }
            case let .claimableAwaitingConfirmations(_, _, amount, confirmationHeight, source):
                if source == .coopClose { continue }
                amountSats += amount
                unlockHeight = max(unlockHeight ?? confirmationHeight, confirmationHeight)
            case let .contentiousClaimable(_, _, amount, _, _, _):
                amountSats += amount
            case let .maybeTimeoutClaimableHtlc(_, _, amount, _, _, _):
                amountSats += amount
            case let .maybePreimageClaimableHtlc(_, _, amount, _, _):
                amountSats += amount
            case let .counterpartyRevokedOutputClaimable(_, _, amount):
                amountSats += amount
            }
        }
        for sweep in pendingSweeps {
            switch sweep {
            case let .pendingBroadcast(_, amount):
                amountSats += amount
            case let .broadcastAwaitingConfirmation(_, _, _, amount):
                amountSats += amount
            case .awaitingThresholdConfirmations:
                break
            }
        }
        guard amountSats > 0 else { return nil }
        return ClosedChannelFunds(
            amountSats: amountSats,
            blocksLeft: unlockHeight.map { $0 > currentHeight ? $0 - currentHeight : 0 }
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

    /// LDK's funds when it tracks a close, else what the on-chain recovery found (#364).
    static func orRecovered(_ ldk: ClosedChannelFunds?, recoveredSats: UInt64) -> ClosedChannelFunds? {
        ldk ?? (recoveredSats > 0 ? ClosedChannelFunds(amountSats: recoveredSats, blocksLeft: nil) : nil)
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
