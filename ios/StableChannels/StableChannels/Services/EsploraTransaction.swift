import Foundation

/// Reads Esplora transaction bodies: `/tx/:txid`, and the elements of `/address/:addr/txs`.
/// Holds the one address-matching rule for recovery, txid resolution and the confirmation address
/// check; what counts as a deposit is decided in `MissedReceiveRecovery`.
enum EsploraTransaction {
    /// `/address/{addr}/txs`: the deposits to `address` per transaction that
    /// `MissedReceiveRecovery` counts. Nil when the body is not a transaction list.
    static func recentReceives(json: Data, address: String, nowSecs: Int64) -> [MissedReceiveRecovery.Receive]? {
        guard let txs = try? JSONSerialization.jsonObject(with: json) as? [[String: Any]] else {
            return nil
        }
        return txs.compactMap { tx in
            guard let txid = tx["txid"] as? String else { return nil }
            let status = tx["status"] as? [String: Any]
            return MissedReceiveRecovery.receive(
                txid: txid,
                sats: satsPaidTo(tx, address: address),
                blockTime: status?["block_time"] as? Int64 ?? 0,
                nowSecs: nowSecs
            )
        }
    }

    /// The outputs of a transaction that pay `address`, compared in normalized form (bech32 case
    /// and surrounding whitespace never change the match). Nil when the body has no output list.
    static func outputsPaying(_ tx: [String: Any], address: String) -> [[String: Any]]? {
        guard let outputs = tx["vout"] as? [[String: Any]] else { return nil }
        let target = PaymentDestinationClassifier.normalizeAddress(address)
        guard !target.isEmpty else { return [] }
        return outputs.filter {
            ($0["scriptpubkey_address"] as? String).map(PaymentDestinationClassifier.normalizeAddress) == target
        }
    }

    /// Sum of the outputs of a transaction that pay `address`. Zero when none do, or when a value
    /// is missing, not positive, or the total is more than can exist (a hostile server must not
    /// trap the app).
    static func satsPaidTo(_ tx: [String: Any], address: String) -> Int64 {
        let values = (outputsPaying(tx, address: address) ?? []).map { $0["value"] as? Int64 ?? 0 }
        guard !values.contains(where: { $0 <= 0 }) else { return 0 }
        return OnchainAmount.total(of: values) ?? 0
    }
}
