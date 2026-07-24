import Foundation

// MARK: - TransactionMatcher

/// Pure matching logic — no actor isolation, no side effects.
/// All state is passed in, so the matcher is a deterministic function
/// of its inputs and trivially testable.
struct TransactionMatcher {
    func match(
        trackedAddresses: Set<String>,
        trackedTxids: Set<String>,
        msg: MempoolWSMessage,
        tx: MempoolWSTransaction
    ) -> (target: String?, isTxid: Bool) {
        // Direct address in response JSON
        if let respAddr = msg.address, trackedAddresses.contains(respAddr) {
            return (respAddr, false)
        }

        // Match output scriptpubkey_address
        if let vouts = tx.vout {
            for vout in vouts {
                if let addr = vout.scriptpubkeyAddress, trackedAddresses.contains(addr) {
                    return (addr, false)
                }
            }
        }

        // Match input txid (outspend of tracked funding txid)
        if let vins = tx.vin {
            for vin in vins {
                if let inputTxid = vin.txid, trackedTxids.contains(inputTxid) {
                    return (inputTxid, true)
                }
            }
        }

        // Match tracked txids directly
        if let respTxid = msg.txid, trackedTxids.contains(respTxid) {
            return (respTxid, true)
        }

        // Match bulk tracked-txs: API keys by tracked txid,
        // value contains the spending txid.
        if let tracked = msg.trackedTxs {
            for (trackedTxid, txData) in tracked {
                if trackedTxids.contains(trackedTxid) && txData.txid == tx.txid {
                    return (trackedTxid, true)
                }
            }
        }

        // Match bulk multi-address-transactions dictionary keys
        if let multi = msg.multiAddressTransactions {
            for (addr, txGroup) in multi {
                guard trackedAddresses.contains(addr) else { continue }
                if (txGroup.mempool?.contains(where: { $0.txid == tx.txid }) == true) ||
                    (txGroup.confirmed?.contains(where: { $0.txid == tx.txid }) == true) {
                    return (addr, false)
                }
            }
        }

        return (nil, false)
    }
}
