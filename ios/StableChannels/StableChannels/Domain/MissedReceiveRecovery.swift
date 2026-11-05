import Foundation

/// Finds deposits to our receive address that the websocket and balance-delta paths both
/// missed (socket down, or detection deferred across an app restart). Pure rules: the explorer
/// body is read by `EsploraTransaction`, the recovery service fetches and the repository records.
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

    /// A deposit of [sats] in [txid] when it counts as one: a canonical txid, at least the minimum
    /// deposit, and received within the window. An unconfirmed one ([blockTime] 0) counts as now.
    static func receive(txid: String, sats: Int64, blockTime: Int64, nowSecs: Int64) -> Receive? {
        guard TradeProtocol.isCanonicalIdentifier(txid) else { return nil }
        let receivedAt = blockTime > 0 ? blockTime : nowSecs
        guard sats >= DepositAttribution.minDepositSats, nowSecs - receivedAt <= windowSeconds else { return nil }
        return Receive(txid: txid, sats: sats, timeSecs: receivedAt)
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
