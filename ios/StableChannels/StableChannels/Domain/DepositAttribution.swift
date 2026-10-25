import Foundation

/// Decides how much of an on-chain balance increase is a new deposit.
///
/// A deposit is seen twice: the WebSocket records it by txid, and the wallet balance rises once
/// the node syncs. Only receives whose transaction reached the wallet since the baseline are part
/// of the increase, so they are tied to it by txid, never by amount: an equal amount does not
/// prove two receives are the same deposit.
enum DepositAttribution {
    /// How a balance increase splits against the receives already recorded for arrived txids.
    struct Outcome: Equatable {
        /// Sats no recorded receive explains: a deposit the WebSocket missed.
        let remainderSats: Int64
        /// Recorded receives whose amount differs from the node's, with the node's amount.
        let amountCorrections: [String: Int64]
        /// Whether any recorded receive was part of the increase.
        let matchedRecordedReceive: Bool
    }

    /// Attributes an increase of [depositSats] to the receives [recorded] (by txid) for arrived
    /// txids, trusting the node's [walletSats] over each row's feed amount.
    static func attribute(
        depositSats: Int64,
        recorded: [String: Int64],
        walletSats: [String: Int64]
    ) -> Outcome {
        Outcome(
            remainderSats: remainderSats(
                depositSats: depositSats,
                recordedSats: accountedSats(recorded: recorded, walletSats: walletSats)
            ),
            amountCorrections: amountCorrections(recorded: recorded, walletSats: walletSats),
            matchedRecordedReceive: !recorded.isEmpty
        )
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
        recorded.map { txid, sats in walletSats[txid] ?? sats }
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
