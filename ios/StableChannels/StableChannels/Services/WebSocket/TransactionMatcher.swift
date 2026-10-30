import Foundation

struct MatchResult: Equatable, Hashable {
    let target: String
    let isTxid: Bool
}

/// Which tracked addresses list each txid in a bulk `multi-address-transactions` payload.
/// Built once per message, so matching every transaction in it stays linear in the payload
/// size instead of rescanning each address group for every transaction.
struct MultiAddressMembership {
    private let addressesByTxid: [String: Set<String>]

    init(message: MempoolWSMessage, trackedAddresses: Set<String>) {
        var index: [String: Set<String>] = [:]
        for (address, group) in message.multiAddressTransactions ?? [:]
            where trackedAddresses.contains(address) {
            for groupTxs in [group.mempool, group.confirmed, group.removed] {
                for tx in groupTxs ?? [] {
                    index[tx.txid, default: []].insert(address)
                }
            }
        }
        addressesByTxid = index
    }

    /// Tracked addresses whose group lists `txid`, in a stable order.
    func addresses(listing txid: String) -> [String] {
        (addressesByTxid[txid] ?? []).sorted()
    }
}

/// Pure matching logic — no actor isolation, no side effects.
/// All state is passed in, so the matcher is a deterministic function
/// of its inputs and trivially testable.
struct TransactionMatcher {
    func matchAll(
        trackedAddresses: Set<String>,
        trackedTxids: Set<String>,
        msg: MempoolWSMessage,
        tx: MempoolWSTransaction,
        membership: MultiAddressMembership? = nil
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

        // Match bulk multi-address-transactions dictionary keys. Callers matching many
        // transactions from one message pass a membership index built once for it.
        let membership = membership ?? MultiAddressMembership(message: msg, trackedAddresses: trackedAddresses)
        for addr in membership.addresses(listing: tx.txid) {
            appendIfNew(MatchResult(target: addr, isTxid: false))
        }

        return results
    }
}
