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
            return "Confirmed"
        } else if raw <= 0 {
            return "0/\(required) confirmed"
        } else {
            return "\(display)/\(required) confirmed"
        }
    }

    var isComplete: Bool { display >= required }
}

struct ConfirmationCalculator {
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
}

/// Pure balance calculator enforcing Single Responsibility by isolating
/// balance aggregation logic from UI state management.
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
}
