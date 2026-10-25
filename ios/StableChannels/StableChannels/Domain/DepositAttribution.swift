import Foundation

/// Decides how much of an on-chain balance increase is a new deposit.
///
/// A deposit is seen twice: the WebSocket records it by txid, and the wallet balance rises once
/// the node syncs. Only receives whose transaction reached the wallet since the baseline are part
/// of the increase, so they are tied to it by txid, never by amount: an equal amount does not
/// prove two receives are the same deposit.
enum DepositAttribution {
    /// Txids whose deposits are inside the increase: the wallet lists them now but did not when
    /// the baseline was taken.
    static func arrivedTxids(wallet: Set<String>, baseline: Set<String>) -> Set<String> {
        wallet.subtracting(baseline)
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
