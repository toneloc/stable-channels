import Foundation

/// Finds deposits to our receive address that the websocket and balance-delta paths both
/// missed (socket down, or detection deferred across an app restart). Parsing and filtering
/// are pure; AppState fetches and the repository records.
enum MissedReceiveRecovery {
    static let windowSeconds: Int64 = 24 * 60 * 60
    static let minSats: Int64 = 1000
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
            guard sats >= minSats, nowSecs - receivedAt <= windowSeconds else { return nil }
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

    /// Sum of the outputs of an Esplora transaction that pay `address`. Zero when none do,
    /// or when the values are invalid or overflow (a hostile server must not trap the app).
    static func satsPaidTo(_ tx: [String: Any], address: String) -> Int64 {
        var total: Int64 = 0
        for output in outputsPaying(tx, address: address) ?? [] {
            guard let value = output["value"] as? Int64, value > 0 else { return 0 }
            let sum = total.addingReportingOverflow(value)
            if sum.overflow { return 0 }
            total = sum.partialValue
        }
        return total
    }

    /// Whether a balance delta is already accounted for: the latest txid seen paying the
    /// current receive address still has a pending row (a confirmed one is an earlier deposit,
    /// so the new delta is not it). Never used to label the delta, since it can hold several
    /// deposits that landed between two checks (#317); the others get their own rows from the
    /// websocket or recovery.
    static func isDeltaAlreadyRecorded(
        lastReceiveTxid: String?,
        lastReceiveTxidAddress: String?,
        receiveAddress: String?,
        txidHasPendingRow: (String) -> Bool
    ) -> Bool {
        guard let txid = lastReceiveTxid, !txid.isEmpty,
              let address = receiveAddress, !address.isEmpty,
              lastReceiveTxidAddress == address else { return false }
        return txidHasPendingRow(txid)
    }
}

/// The balance-delta path writes one placeholder for everything that arrived between two
/// checks, so two deposits landing together share one row until each gets its own txid row
/// (#317). Such a placeholder is dropped once two or more of those rows add up to it.
enum MergedReceivePolicy {
    /// How long after a merged placeholder its deposits' own rows may be recorded. Recovery
    /// runs every 5 minutes, so this allows a retry; later rows are other deposits.
    static let windowSeconds: Int64 = 600
    /// The rows that split a placeholder are recorded soon after it, so the earliest few are
    /// enough and keep the subset search small.
    static let maxRowsPerPlaceholder = 16

    struct Row: Equatable {
        let id: Int64
        let amountMsat: Int64
        let createdAt: Int64
    }

    /// Placeholders that two or more `receives` (txid rows on the same address, recorded
    /// within the window after the placeholder) add up to exactly. A single later row never
    /// matches: that is a separate deposit.
    static func mergedPlaceholderIds(placeholders: [Row], receives: [Row]) -> [Int64] {
        let ordered = receives.sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }
        return placeholders.compactMap { placeholder in
            let later = ordered.lazy
                .filter {
                    $0.createdAt >= placeholder.createdAt
                        && $0.createdAt <= placeholder.createdAt + windowSeconds
                }
                .prefix(maxRowsPerPlaceholder)
                .map(\.amountMsat)
            return twoOrMoreSumTo(Array(later), target: placeholder.amountMsat) ? placeholder.id : nil
        }
    }

    /// Whether two or more of `amounts` add up to exactly `target`.
    static func twoOrMoreSumTo(_ amounts: [Int64], target: Int64) -> Bool {
        guard target > 0 else { return false }
        var single = Set<Int64>()
        var multiple = Set<Int64>()
        for amount in amounts where amount > 0 && amount <= target {
            let extended = single.union(multiple).map { $0 + amount }.filter { $0 <= target }
            multiple.formUnion(extended)
            single.insert(amount)
        }
        return multiple.contains(target)
    }
}

/// Gives txid-less pending receive placeholders their txid from the wallet's own payment
/// list. Covers deposits whose address lookup gave up, which would otherwise stay at 0
/// confirmations forever because the confirmation poller only tracks rows with a txid.
enum ReceiveTxidAdoption {
    /// How far before a placeholder's creation the wallet may first have seen its deposit:
    /// the wallet learns of a transaction, then the balance change creates the placeholder.
    static let slackSeconds: Int64 = 600

    /// A txid-less pending receive. Only rows with an address qualify: an amount match alone
    /// does not prove which transaction paid them.
    struct Placeholder: Equatable {
        let id: Int64
        let address: String
        let amountMsat: Int64
        let createdAt: Int64
    }

    /// An inbound on-chain payment from the wallet, and when the wallet last updated it.
    struct WalletReceive: Equatable {
        let txid: String
        let amountMsat: Int64
        let seenAtSecs: Int64
    }

    /// A txid that may belong to a placeholder. Also the key for remembered rejections.
    struct Candidate: Hashable {
        let rowId: Int64
        let txid: String
    }

    /// Exact-amount pairs for txids not yet on any row. A row created more than the slack
    /// after the wallet saw the tx cannot be this deposit (an older payment to a reused
    /// address), so it is skipped. The caller checks each txid pays the row's address.
    static func candidates(
        placeholders: [Placeholder],
        walletReceives: [WalletReceive],
        recordedTxids: Set<String>
    ) -> [Candidate] {
        let byAmount = Dictionary(grouping: placeholders, by: \.amountMsat)
        var result: [Candidate] = []
        for receive in walletReceives where !recordedTxids.contains(receive.txid) {
            for row in byAmount[receive.amountMsat] ?? []
                where row.createdAt <= receive.seenAtSecs + slackSeconds {
                result.append(Candidate(rowId: row.id, txid: receive.txid))
            }
        }
        return result
    }

    /// Rows paid by exactly one distinct txid. A reused address can hold several same-amount
    /// payments; without a way to tell them apart, the row is left unresolved.
    static func adoptions(paying: [Candidate]) -> [Int64: String] {
        Dictionary(grouping: paying, by: \.rowId).compactMapValues { candidates in
            let txids = Set(candidates.map(\.txid))
            return txids.count == 1 ? txids.first : nil
        }
    }
}
