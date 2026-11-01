import Foundation

/// Finds deposits to our receive address that the websocket and balance-delta paths both
/// missed (socket down, or detection deferred across an app restart). Parsing and filtering
/// are pure; AppState fetches and the repository records.
enum MissedReceiveRecovery {
    static let windowSeconds: Int64 = 24 * 60 * 60
    static let checkIntervalSeconds: Int64 = 300

    struct Receive: Equatable, Sendable {
        let txid: String
        let sats: Int64
        let timeSecs: Int64
    }

    static func isCheckDue(lastCheckSecs: Int64, nowSecs: Int64) -> Bool {
        nowSecs - lastCheckSecs >= checkIntervalSeconds
    }

    /// Esplora `/address/{addr}/txs`: sum of outputs paying `address` per tx, within the
    /// window. Unconfirmed txs count as received now. Nil when the body is not a tx list.
    static func recentReceives(json: Data, address: String, nowSecs: Int64) -> [Receive]? {
        guard let txs = try? JSONSerialization.jsonObject(with: json) as? [[String: Any]] else {
            return nil
        }
        return txs.compactMap { tx in
            guard let txid = tx["txid"] as? String, TradeProtocol.isCanonicalIdentifier(txid) else {
                return nil
            }
            let sats = satsPaidTo(tx, address: address)
            let status = tx["status"] as? [String: Any]
            let blockTime = status?["block_time"] as? Int64 ?? 0
            let receivedAt = blockTime > 0 ? blockTime : nowSecs
            guard sats >= DepositAttribution.minDepositSats, nowSecs - receivedAt <= windowSeconds else { return nil }
            return Receive(txid: txid, sats: sats, timeSecs: receivedAt)
        }
    }

    /// The outputs of an Esplora transaction that pay `address`, compared in normalized form
    /// (bech32 case and surrounding whitespace never change the match). Nil when the body has no
    /// output list. The one address-matching rule for recovery, txid resolution and the
    /// confirmation address check.
    static func outputsPaying(_ tx: [String: Any], address: String) -> [[String: Any]]? {
        guard let outputs = tx["vout"] as? [[String: Any]] else { return nil }
        let target = PaymentDestinationClassifier.normalizeAddress(address)
        guard !target.isEmpty else { return [] }
        return outputs.filter {
            ($0["scriptpubkey_address"] as? String).map(PaymentDestinationClassifier.normalizeAddress) == target
        }
    }

    /// Sum of the outputs of an Esplora transaction that pay `address`. Zero when none do, or
    /// when a value is missing, not positive, or the total is more than can exist (a hostile
    /// server must not trap the app).
    static func satsPaidTo(_ tx: [String: Any], address: String) -> Int64 {
        let values = (outputsPaying(tx, address: address) ?? []).map { $0["value"] as? Int64 ?? 0 }
        guard !values.contains(where: { $0 <= 0 }) else { return 0 }
        return OnchainAmount.total(of: values) ?? 0
    }
}

/// Gives pending receive placeholders their txid from the wallet's own payment list. Covers
/// deposits whose address lookup gave up, which would otherwise stay at 0 confirmations forever
/// because the confirmation poller only tracks rows with a txid. Only a txid recorded as the
/// placeholder's candidate qualifies: an equal amount alone never proves which transaction paid it.
enum ReceiveTxidAdoption {
    /// A pending placeholder with an address, and the txids its balance jump could be.
    struct Placeholder: Equatable {
        let id: Int64
        let address: String
        let amountMsat: Int64
        var candidateTxids: Set<String> = []
    }

    /// A txid that may belong to a placeholder. Also the key for remembered rejections.
    struct Candidate: Hashable {
        let rowId: Int64
        let txid: String
    }

    /// Candidate txids the wallet lists at exactly the placeholder's amount that no row records
    /// yet. The caller checks each txid pays the row's address.
    static func candidates(
        placeholders: [Placeholder],
        wallet: WalletInboundOnchain,
        recordedTxids: Set<String>
    ) -> [Candidate] {
        placeholders.flatMap { row in
            row.candidateTxids.sorted().compactMap { txid -> Candidate? in
                guard !recordedTxids.contains(txid),
                      let msat = wallet.sats[txid].flatMap(OnchainAmount.msats(fromSats:)),
                      Int64(msat) == row.amountMsat else { return nil }
                return Candidate(rowId: row.id, txid: txid)
            }
        }
    }

    /// Rows paid by exactly one distinct txid. Two candidates of the full amount cannot both be
    /// the deposit; without a way to tell them apart, the row is left unresolved.
    static func adoptions(paying: [Candidate]) -> [Int64: String] {
        Dictionary(grouping: paying, by: \.rowId).compactMapValues { candidates in
            let txids = Set(candidates.map(\.txid))
            return txids.count == 1 ? txids.first : nil
        }
    }
}
