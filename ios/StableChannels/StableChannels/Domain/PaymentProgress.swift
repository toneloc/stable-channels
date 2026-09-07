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
        onchainPaymentTypes.contains(paymentType)
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

    struct PendingOutboundSend: Equatable, Sendable {
        var amountSats: UInt64
        var isSendAll: Bool
        var baselineOnchainSats: UInt64
        var timestampSecs: Int64
        var txids: [String]

        var txid: String? { txids.first }

        init(
            amountSats: UInt64 = 0,
            isSendAll: Bool = false,
            baselineOnchainSats: UInt64 = 0,
            timestampSecs: Int64 = Int64(Date().timeIntervalSince1970),
            txids: [String] = []
        ) {
            self.amountSats = amountSats
            self.isSendAll = isSendAll
            self.baselineOnchainSats = baselineOnchainSats
            self.timestampSecs = timestampSecs
            self.txids = txids
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
        var updatedTxids = currentPending.txids
        if let txid, !txid.isEmpty, !updatedTxids.contains(txid) {
            updatedTxids.append(txid)
        }
        if isSendAll {
            return PendingOutboundSend(
                amountSats: currentPending.amountSats + currentOnchain,
                isSendAll: true,
                baselineOnchainSats: baseline,
                timestampSecs: timestampSecs,
                txids: updatedTxids
            )
        } else {
            return PendingOutboundSend(
                amountSats: currentPending.amountSats + amountSats,
                isSendAll: false,
                baselineOnchainSats: baseline,
                timestampSecs: timestampSecs,
                txids: updatedTxids
            )
        }
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
        if pending.amountSats > 0 {
            let rawDrop = (rawOnchain < pending.baselineOnchainSats) ? (pending.baselineOnchainSats - rawOnchain) : 0
            let pendingToDeduct = pending.amountSats > rawDrop ? (pending.amountSats - rawDrop) : 0
            let onchain = rawOnchain >= pendingToDeduct ? rawOnchain - pendingToDeduct : 0
            let spendable = rawSpendable >= pendingToDeduct ? rawSpendable - pendingToDeduct : 0
            return (onchain, spendable)
        }
        return (rawOnchain, rawSpendable)
    }

    /// Resolves pending outbound send state against a fresh raw on-chain balance observation.
    /// Once the raw balance proves the spend has been incorporated, or authoritative transaction
    /// status confirms incorporation / failure, pending state clears.
    /// Fails closed during extended indexer/node outages to prevent re-exposing spent funds.
    static func resolvePendingOutboundSend(
        rawOnchain: UInt64,
        pending: PendingOutboundSend,
        currentTimeSecs _: Int64 = Int64(Date().timeIntervalSince1970),
        expirySecs _: Int64 = defaultPendingExpirySecs,
        isTxConfirmed: ((String) -> Bool)? = nil,
        isTxFailed: ((String) -> Bool)? = nil
    ) -> PendingOutboundSend {
        guard pending.amountSats > 0 || pending.isSendAll else {
            return pending
        }

        // 1. Authoritative transaction failure check:
        if let isTxFailed, !pending.txids.isEmpty, pending.txids.allSatisfy({ isTxFailed($0) }) {
            return PendingOutboundSend(timestampSecs: 0)
        }

        // 2. Authoritative transaction confirmation check:
        if let isTxConfirmed, !pending.txids.isEmpty, pending.txids.allSatisfy({ isTxConfirmed($0) }) {
            return PendingOutboundSend(timestampSecs: 0)
        }

        // 3. Raw balance drop check:
        if pending.isSendAll {
            if rawOnchain == 0 {
                return PendingOutboundSend(timestampSecs: 0)
            }
            return pending
        }
        if pending.amountSats > 0 {
            let expectedRemaining = pending.baselineOnchainSats >= pending.amountSats
                ? pending.baselineOnchainSats - pending.amountSats
                : 0
            if rawOnchain <= expectedRemaining {
                return PendingOutboundSend(timestampSecs: 0)
            }
            // Fail closed beyond expiry: retain deduction until authoritative reconciliation
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
