import Foundation

enum ConfirmationPolicy {
    static let requiredConfirmations = 3
}

struct ConfirmationProgress: Equatable {
    let raw: Int // actual confirmations (may exceed requiredConfirmations)
    let display: Int // capped at requiredConfirmations for UI

    var label: String { "\(display)/\(ConfirmationPolicy.requiredConfirmations) done" }
    var isComplete: Bool { display >= ConfirmationPolicy.requiredConfirmations }
}

struct ConfirmationCalculator {
    func progress(for txBlockHeight: UInt32, currentBlockHeight: UInt32) -> ConfirmationProgress {
        let confs = Int(currentBlockHeight) - Int(txBlockHeight) + 1
        let clamped = min(max(confs, 0), ConfirmationPolicy.requiredConfirmations)
        return ConfirmationProgress(raw: confs, display: clamped)
    }
}

extension PaymentRecord {
    var shouldShowConfirmationProgress: Bool {
        switch paymentType {
        case "onchain", "splice_in", "splice_out", "channel_close":
            return true
        default:
            return false
        }
    }

    var isOnchainConfirmed: Bool {
        (txBlockHeight ?? 0) > 0
    }
}
