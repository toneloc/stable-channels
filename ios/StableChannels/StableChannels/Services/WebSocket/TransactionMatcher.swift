import Foundation

struct MatchResult: Equatable, Hashable {
    let target: String
    let isTxid: Bool
}

/// Pure matching logic — no actor isolation, no side effects.
/// All state is passed in, so the matcher is a deterministic function
/// of its inputs and trivially testable.
struct TransactionMatcher {
    func matchAll(
        trackedAddresses: Set<String>,
        trackedTxids: Set<String>,
        msg: MempoolWSMessage,
        tx: MempoolWSTransaction
    ) -> [MatchResult] {
        var results = [MatchResult]()
        var seen = Set<MatchResult>()

        func appendIfNew(_ res: MatchResult) {
            if seen.insert(res).inserted {
                results.append(res)
            }
        }

        // Direct address in response JSON
        if let respAddr = msg.address, trackedAddresses.contains(respAddr) {
            appendIfNew(MatchResult(target: respAddr, isTxid: false))
        }

        // Match output scriptpubkey_address
        if let vouts = tx.vout {
            for vout in vouts {
                if let addr = vout.scriptpubkeyAddress, trackedAddresses.contains(addr) {
                    appendIfNew(MatchResult(target: addr, isTxid: false))
                }
            }
        }

        // Match input txid (outspend of tracked funding txid)
        if let vins = tx.vin {
            for vin in vins {
                if let inputTxid = vin.txid, trackedTxids.contains(inputTxid) {
                    appendIfNew(MatchResult(target: inputTxid, isTxid: true))
                }
            }
        }

        // Match tracked txids directly
        if let respTxid = msg.txid, trackedTxids.contains(respTxid) {
            appendIfNew(MatchResult(target: respTxid, isTxid: true))
        }

        // Match bulk multi-address-transactions dictionary keys
        if let multi = msg.multiAddressTransactions {
            for (addr, txGroup) in multi {
                guard trackedAddresses.contains(addr) else { continue }
                if (txGroup.mempool?.contains(where: { $0.txid == tx.txid }) == true) ||
                    (txGroup.confirmed?.contains(where: { $0.txid == tx.txid }) == true) ||
                    (txGroup.removed?.contains(where: { $0.txid == tx.txid }) == true) {
                    appendIfNew(MatchResult(target: addr, isTxid: false))
                }
            }
        }

        return results
    }
}
