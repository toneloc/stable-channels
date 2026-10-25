import Foundation

/// Decides how much of an on-chain balance increase is a new deposit.
///
/// A deposit is seen twice: the WebSocket records it by txid, and the wallet balance rises once
/// the node syncs. Only receives whose transaction reached the wallet since the baseline are part
/// of the increase, so they are tied to it by txid, never by amount: an equal amount does not
/// prove two receives are the same deposit.
enum DepositAttribution {
    /// Smaller balance increases and receives are fee-estimate noise, not deposits.
    static let minDepositSats: Int64 = 1000

    /// How a balance increase splits against the receives already recorded for arrived txids.
    struct Outcome: Equatable {
        /// Sats no recorded receive explains: a deposit the WebSocket missed.
        let remainderSats: Int64
        /// Recorded receives whose amount differs from the node's, with the node's amount.
        let amountCorrections: [String: Int64]
        /// Whether any recorded receive was part of the increase.
        let matchedRecordedReceive: Bool
        /// The transaction the remainder is, when exactly one arrived txid has no recorded receive
        /// and the node's amount for it is the remainder.
        let missedTxid: String?
    }

    /// Attributes an increase of [depositSats] to the receives [recorded] (by txid) for the [arrived]
    /// txids, trusting the node's [walletSats] over each row's feed amount.
    static func attribute(
        depositSats: Int64,
        arrived: Set<String>,
        recorded: [String: Int64],
        walletSats: [String: Int64]
    ) -> Outcome {
        let remainder = remainderSats(
            depositSats: depositSats,
            recordedSats: accountedSats(recorded: recorded, walletSats: walletSats)
        )
        return Outcome(
            remainderSats: remainder,
            amountCorrections: amountCorrections(recorded: recorded, walletSats: walletSats),
            matchedRecordedReceive: !recorded.isEmpty,
            missedTxid: missedTxid(
                remainderSats: remainder,
                unrecorded: arrived.subtracting(recorded.keys),
                walletSats: walletSats
            )
        )
    }

    /// The txid of the deposit the WebSocket missed, when it is the only [unrecorded] arrival and the
    /// node's amount for it is the whole remainder. Several unrecorded arrivals cannot be told apart.
    static func missedTxid(remainderSats: Int64, unrecorded: Set<String>, walletSats: [String: Int64]) -> String? {
        guard remainderSats > 0, unrecorded.count == 1, let txid = unrecorded.first,
              walletSats[txid] == remainderSats else { return nil }
        return txid
    }

    /// Txids whose deposits are inside the increase: the wallet lists them now but did not when
    /// the baseline was taken.
    static func arrivedTxids(wallet: Set<String>, baseline: Set<String>) -> Set<String> {
        wallet.subtracting(baseline)
    }

    /// What the recorded receives for arrived txids count against the increase. A row's amount came
    /// from the WebSocket feed, so the node's own amount for the txid replaces it; a txid the node
    /// lists without an amount keeps the row's, so its receive is still not recorded twice.
    static func accountedSats(recorded: [String: Int64], walletSats: [String: Int64]) -> [Int64] {
        recorded.map { txid, sats in trustedSats(txid: txid, feedSats: sats, walletSats: walletSats) }
    }

    /// The amount to trust for [txid]: the node's once it lists one, otherwise the feed's.
    static func trustedSats(txid: String, feedSats: Int64, walletSats: [String: Int64]) -> Int64 {
        walletSats[txid] ?? feedSats
    }

    /// Recorded receives whose amount differs from the node's, with the node's amount to keep.
    static func amountCorrections(recorded: [String: Int64], walletSats: [String: Int64]) -> [String: Int64] {
        recorded.reduce(into: [:]) { corrections, row in
            if let sats = walletSats[row.key], sats != row.value { corrections[row.key] = sats }
        }
    }

    /// The part of the increase that the receives already recorded for arrived txids don't explain.
    static func remainderSats(depositSats: Int64, recordedSats: [Int64]) -> Int64 {
        let recorded = recordedSats.reduce(Int64(0)) { sum, sats in
            let (total, overflow) = sum.addingReportingOverflow(max(0, sats))
            return overflow ? .max : total
        }
        return max(0, depositSats - recorded)
    }
}

/// The on-chain balance deposit detection has accounted for, with the inbound txids inside it: a
/// later increase never counts them again.
struct OnchainDepositBaseline: Equatable {
    var sats: UInt64
    /// nil when no list was cached with the balance (first launch, or an install from before the
    /// list existed).
    var txids: Set<String>?
    /// When tracking began. While [txids] is unknown, only receives recorded since then can be
    /// inside an increase.
    var since = Date()
    /// Counted receives the explorer confirmed gone, with their sats. They stay counted until the node
    /// syncs past them: a poll before that still sees the original in the balance.
    private(set) var removed: [String: Int64] = [:]

    /// Moves the baseline to [sats], with the txids the wallet lists now inside it.
    mutating func advance(to sats: UInt64, wallet: Set<String>) {
        self.sats = sats
        txids = wallet
    }

    /// Whether the baseline already counts [txid].
    func counts(_ txid: String) -> Bool {
        txids?.contains(txid) ?? false
    }

    /// A receive of [removedSats] left the mempool. Only one the baseline counted affects it.
    mutating func markRemoved(txid: String, sats removedSats: Int64) {
        guard counts(txid) else { return }
        removed[txid] = max(0, removedSats)
    }

    /// Takes marked removals out once the node has synced past them, so the transaction replacing
    /// one is a new increase. A balance below the baseline already left them out.
    mutating func settleRemovals(wallet: Set<String>, balance: UInt64) {
        guard !removed.isEmpty, balance < sats || wallet != txids else { return }
        if balance >= sats {
            sats -= min(sats, UInt64(removed.values.reduce(0, +)))
        }
        removed = [:]
    }
}
