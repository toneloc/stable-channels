import Foundation

struct MatchResult: Equatable {
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

        // Direct address in response JSON
        if let respAddr = msg.address, trackedAddresses.contains(respAddr) {
            results.append(MatchResult(target: respAddr, isTxid: false))
        }

        // Match output scriptpubkey_address
        if let vouts = tx.vout {
            for vout in vouts {
                if let addr = vout.scriptpubkeyAddress, trackedAddresses.contains(addr) {
                    let res = MatchResult(target: addr, isTxid: false)
                    if !results.contains(res) { results.append(res) }
                }
            }
        }

        // Match input txid (outspend of tracked funding txid)
        if let vins = tx.vin {
            for vin in vins {
                if let inputTxid = vin.txid, trackedTxids.contains(inputTxid) {
                    let res = MatchResult(target: inputTxid, isTxid: true)
                    if !results.contains(res) { results.append(res) }
                }
            }
        }

        // Match tracked txids directly
        if let respTxid = msg.txid, trackedTxids.contains(respTxid) {
            let res = MatchResult(target: respTxid, isTxid: true)
            if !results.contains(res) { results.append(res) }
        }

        // Match bulk multi-address-transactions dictionary keys
        if let multi = msg.multiAddressTransactions {
            for (addr, txGroup) in multi {
                guard trackedAddresses.contains(addr) else { continue }
                if (txGroup.mempool?.contains(where: { $0.txid == tx.txid }) == true) ||
                    (txGroup.confirmed?.contains(where: { $0.txid == tx.txid }) == true) ||
                    (txGroup.removed?.contains(where: { $0.txid == tx.txid }) == true) {
                    let res = MatchResult(target: addr, isTxid: false)
                    if !results.contains(res) { results.append(res) }
                }
            }
        }

        return results
    }
}
