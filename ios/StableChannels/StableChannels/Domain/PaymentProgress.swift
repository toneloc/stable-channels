import Foundation

enum ConfirmationPolicy {
    static let defaultRequiredConfirmations = 6
    static let spliceRequiredConfirmations = 1

    static var requiredConfirmations: Int { defaultRequiredConfirmations }

    static func requiredConfirmations(for paymentType: String) -> Int {
        switch paymentType {
        case "splice_in", "splice_out":
            return spliceRequiredConfirmations
        default:
            return defaultRequiredConfirmations
        }
    }
}

struct ConfirmationProgress: Equatable {
    let raw: Int
    let display: Int
    let required: Int

    init(raw: Int, display: Int, required: Int = ConfirmationPolicy.defaultRequiredConfirmations) {
        self.raw = raw
        self.display = display
        self.required = required
    }

    var label: String {
        if isComplete {
            return String(localized: "status_confirmed", defaultValue: "Confirmed")
        } else if raw <= 0 {
            return "0/\(required) confirmed"
        } else {
            return "\(display)/\(required) confirmed"
        }
    }

    var isComplete: Bool { display >= required }
}

protocol ConfirmationCalculating: Sendable {
    func progress(
        for txBlockHeight: UInt32,
        currentBlockHeight: UInt32,
        required: Int
    ) -> ConfirmationProgress
}

struct ConfirmationCalculator: ConfirmationCalculating {
    func progress(
        for txBlockHeight: UInt32,
        currentBlockHeight: UInt32,
        required: Int = ConfirmationPolicy.defaultRequiredConfirmations
    ) -> ConfirmationProgress {
        let confs = Int(currentBlockHeight) - Int(txBlockHeight) + 1
        let raw = max(confs, 0)
        let display = min(raw, required)
        return ConfirmationProgress(raw: raw, display: display, required: required)
    }
}

private let onchainPaymentTypes: Set<String> = ["onchain", "splice_in", "splice_out", "channel_close"]

extension PaymentRecord {
    var shouldShowConfirmationProgress: Bool {
        // Only rows with a live progress signal get the counting badge; a
        // failed or expired splice must fall through to the status label
        // ("Failed") instead of showing "0/N confirmed" forever. Mirrors
        // Android's HistoryScreen.shouldShowConfirmationProgress().
        onchainPaymentTypes.contains(paymentType)
            && (status == "pending" || Int(confirmations) > 0)
    }

    var isOnchainConfirmed: Bool {
        (txBlockHeight ?? 0) > 0
    }

    var confirmationProgress: ConfirmationProgress {
        let required = ConfirmationPolicy.requiredConfirmations(for: paymentType)
        let raw = Int(confirmations)
        let display = min(max(raw, 0), required)
        return ConfirmationProgress(raw: raw, display: display, required: required)
    }
}

/// Pure balance calculator isolating balance aggregation logic from UI state management.
enum BalanceCalculator {
    struct ChannelState: Equatable {
        var hasReadyChannel: Bool
        var hasAnyChannel: Bool = false
        var isChannelClosing: Bool = false
        var isOpeningChannel: Bool = false
        var isSweeping: Bool = false
    }

    static func calculateTotalBalance(
        lightning: UInt64,
        onchain: UInt64,
        pendingSweep: UInt64 = 0,
        channelState: ChannelState
    ) -> UInt64 {
        if channelState.isChannelClosing {
            return onchain
        }
        if channelState.isOpeningChannel {
            return lightning > 0 ? lightning : onchain
        }
        if channelState.isSweeping {
            return lightning
        }
        // If no ready channel and no channels exist, lightning balance contains stale claimables from
        // closed channels — never count it, even when onchainBalanceSats reaches 0 (Issue #260).
        if !channelState.hasReadyChannel && !channelState.hasAnyChannel {
            return onchain + pendingSweep
        }
        return lightning + onchain
    }

    /// A single pending broadcast entry pairing a transaction id with its sent amount.
    /// Enables per-transaction resolution so mixed succeeded/failed batches release
    /// only the resolved portion instead of blocking the entire aggregate.
    struct TxEntry: Equatable, Sendable {
        let txid: String
        let amountSats: UInt64
    }

    struct PendingOutboundSend: Equatable, Sendable {
        var isSendAll: Bool
        var baselineOnchainSats: UInt64
        var timestampSecs: Int64
        var entries: [TxEntry]

        /// Aggregate pending amount across all unresolved entries.
        var amountSats: UInt64 { entries.reduce(0) { $0 + $1.amountSats } }

        /// All pending txids for predicate checks.
        var txids: [String] { entries.map(\.txid) }

        /// First broadcast txid, if any.
        var txid: String? { entries.first?.txid }

        init(
            amountSats: UInt64 = 0,
            isSendAll: Bool = false,
            baselineOnchainSats: UInt64 = 0,
            timestampSecs: Int64 = Int64(Date().timeIntervalSince1970),
            txids: [String] = [],
            entries: [TxEntry]? = nil
        ) {
            self.isSendAll = isSendAll
            self.baselineOnchainSats = baselineOnchainSats
            self.timestampSecs = timestampSecs
            // If explicit entries are provided, use them; otherwise derive from legacy fields.
            if let entries {
                self.entries = entries
            } else if !txids.isEmpty, amountSats > 0 {
                // Backward compatibility: distribute aggregate evenly across txids.
                let perTx = amountSats / UInt64(txids.count)
                let remainder = amountSats % UInt64(txids.count)
                self.entries = txids.enumerated().map { i, tid in
                    TxEntry(txid: tid, amountSats: perTx + (UInt64(i) < remainder ? 1 : 0))
                }
            } else if amountSats > 0 {
                // No txid yet (pre-broadcast state or legacy record without txid tracking).
                self.entries = [TxEntry(txid: "", amountSats: amountSats)]
            } else {
                self.entries = []
            }
        }
    }

    static let defaultPendingExpirySecs: Int64 = 600

    /// Records an immediate outbound send broadcast and returns the updated pending state.
    static func recordBroadcast(
        currentPending: PendingOutboundSend,
        amountSats: UInt64,
        isSendAll: Bool,
        currentOnchain: UInt64,
        timestampSecs: Int64 = Int64(Date().timeIntervalSince1970),
        txid: String? = nil
    ) -> PendingOutboundSend {
        let baseline = currentPending.baselineOnchainSats == 0 ? currentOnchain : currentPending.baselineOnchainSats
        var updatedEntries = currentPending.entries
        let sendAmount = isSendAll ? currentOnchain : amountSats
        if let txid, !txid.isEmpty {
            if !updatedEntries.contains(where: { $0.txid == txid }) {
                updatedEntries.append(TxEntry(txid: txid, amountSats: sendAmount))
            }
        } else {
            // No txid available yet; append an anonymous entry.
            updatedEntries.append(TxEntry(txid: "", amountSats: sendAmount))
        }
        return PendingOutboundSend(
            isSendAll: isSendAll || currentPending.isSendAll,
            baselineOnchainSats: baseline,
            timestampSecs: timestampSecs,
            entries: updatedEntries
        )
    }

    /// Derives user-facing on-chain and spendable balances by subtracting any pending
    /// outbound send that has not yet been incorporated into LDK/BDK's raw wallet view.
    /// If raw balance has already dropped below baseline, only the unincorporated portion
    /// of pending sends is subtracted to avoid double-deductions during partial syncs.
    static func calculateEffectiveBalances(
        rawOnchain: UInt64,
        rawSpendable: UInt64,
        pending: PendingOutboundSend
    ) -> (onchain: UInt64, spendable: UInt64) {
        if pending.isSendAll {
            return (0, 0)
        }
        let amount = pending.amountSats
        if amount > 0 {
            let rawDrop = (rawOnchain < pending.baselineOnchainSats) ? (pending.baselineOnchainSats - rawOnchain) : 0
            let pendingToDeduct = amount > rawDrop ? (amount - rawDrop) : 0
            let onchain = rawOnchain >= pendingToDeduct ? rawOnchain - pendingToDeduct : 0
            let spendable = rawSpendable >= pendingToDeduct ? rawSpendable - pendingToDeduct : 0
            return (onchain, spendable)
        }
        return (rawOnchain, rawSpendable)
    }

    /// Resolves pending outbound send state against a fresh raw on-chain balance observation.
    /// Performs per-txid resolution: transactions whose authoritative status is known
    /// (incorporated or failed) are removed individually, allowing partial clearing of
    /// mixed-status batches instead of all-or-nothing.
    /// Fails closed during extended indexer/node outages to prevent re-exposing spent funds.
    static func resolvePendingOutboundSend(
        rawOnchain: UInt64,
        pending: PendingOutboundSend,
        currentTimeSecs _: Int64 = Int64(Date().timeIntervalSince1970),
        expirySecs _: Int64 = defaultPendingExpirySecs,
        isTxIncorporated: ((String) -> Bool)? = nil,
        isTxFailed: ((String) -> Bool)? = nil,
        isTxConfirmed: ((String) -> Bool)? = nil
    ) -> PendingOutboundSend {
        guard pending.amountSats > 0 || pending.isSendAll else {
            return pending
        }

        let incorporated = isTxIncorporated ?? isTxConfirmed

        // 1. Per-txid resolution: remove entries whose txid has a terminal or incorporated status.
        var unresolvedEntries = pending.entries
        if !unresolvedEntries.isEmpty {
            unresolvedEntries = unresolvedEntries.filter { entry in
                guard !entry.txid.isEmpty else { return true }
                // Failed transactions: release the deduction (funds were never spent).
                if let isTxFailed, isTxFailed(entry.txid) { return false }
                // Incorporated transactions: wallet already reflects the spend.
                if let incorporated, incorporated(entry.txid) { return false }
                return true
            }
            // If all entries resolved, clear the entire record.
            if unresolvedEntries.isEmpty {
                return PendingOutboundSend(timestampSecs: 0)
            }
            // If some entries resolved, check if we can return a reduced pending record.
            if unresolvedEntries.count < pending.entries.count {
                let resolved = PendingOutboundSend(
                    isSendAll: pending.isSendAll,
                    baselineOnchainSats: pending.baselineOnchainSats,
                    timestampSecs: pending.timestampSecs,
                    entries: unresolvedEntries
                )
                // Re-check raw balance drop against the reduced aggregate.
                return resolveByBalanceDrop(rawOnchain: rawOnchain, pending: resolved)
            }
        }

        // 2. No per-txid resolution occurred; check raw balance drop.
        return resolveByBalanceDrop(rawOnchain: rawOnchain, pending: pending)
    }

    /// Checks whether the raw on-chain balance has dropped enough to account for the
    /// remaining pending deduction. Pure helper for resolvePendingOutboundSend.
    private static func resolveByBalanceDrop(
        rawOnchain: UInt64,
        pending: PendingOutboundSend
    ) -> PendingOutboundSend {
        if pending.isSendAll {
            if rawOnchain == 0 {
                return PendingOutboundSend(timestampSecs: 0)
            }
            return pending
        }
        let amount = pending.amountSats
        if amount > 0 {
            let expectedRemaining = pending.baselineOnchainSats >= amount
                ? pending.baselineOnchainSats - amount
                : 0
            if rawOnchain <= expectedRemaining {
                return PendingOutboundSend(timestampSecs: 0)
            }
            // Fail closed: retain deduction until authoritative reconciliation.
            return pending
        }
        return pending
    }

    /// Pure helper to evaluate if a background wallet sync completion owns the active send generation
    /// and succeeded, preventing older out-of-order syncs from clearing newer pending broadcasts.
    static func shouldClearPendingOnSyncCompletion(
        expectedGeneration: Int64,
        currentGeneration: Int64,
        syncSuccess: Bool
    ) -> Bool {
        return syncSuccess && expectedGeneration == currentGeneration
    }
}
