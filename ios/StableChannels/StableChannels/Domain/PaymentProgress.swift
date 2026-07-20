import Foundation

enum ConfirmationPolicy {
    static let requiredConfirmations = 6
    // Bitcoin mainnet = 6, Liquid = 2 — bump when adding network-specific policies
}

struct ConfirmationProgress: Equatable {
    let raw: Int
    let display: Int

    var label: String { "\(display)/\(ConfirmationPolicy.requiredConfirmations) done" }
    var isComplete: Bool { display == ConfirmationPolicy.requiredConfirmations }
}

struct ConfirmationCalculator {
    func progress(for txBlockHeight: UInt32, currentBlockHeight: UInt32) -> ConfirmationProgress {
        let confs = Int(currentBlockHeight) - Int(txBlockHeight) + 1
        let raw = max(confs, 0)
        let display = min(raw, ConfirmationPolicy.requiredConfirmations)
        return ConfirmationProgress(raw: raw, display: display)
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
