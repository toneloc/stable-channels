import Foundation

/// An on-chain receive the WebSocket recorded before the wallet balance showed it.
struct AwaitingReceive: Equatable {
    let rowId: Int64
    let sats: Int64
}

/// Decides how much of an on-chain balance increase is a new deposit.
///
/// A deposit is seen twice: the WebSocket records it by txid, and the wallet balance rises once
/// the node syncs. Receives the WebSocket already recorded are taken out of the increase, so the
/// balance fallback only records what the WebSocket missed and no deposit is counted twice.
enum DepositAttribution {
    struct Result: Equatable {
        /// WebSocket receives this increase accounts for.
        let absorbedRowIds: [Int64]
        /// The part of the increase no recorded receive explains.
        let remainderSats: Int64
    }

    /// Takes `awaiting` receives, oldest first, while they fit in `depositSats`.
    static func attribute(depositSats: Int64, awaiting: [AwaitingReceive]) -> Result {
        var remainder = depositSats
        var absorbed: [Int64] = []
        for receive in awaiting.sorted(by: { $0.rowId < $1.rowId }) where receive.sats <= remainder {
            remainder -= receive.sats
            absorbed.append(receive.rowId)
        }
        return Result(absorbedRowIds: absorbed, remainderSats: remainder)
    }
}
