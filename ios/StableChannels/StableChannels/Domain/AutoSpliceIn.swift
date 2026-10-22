import Foundation

/// Decides when confirmed on-chain funds are moved into the channel without the user tapping Move.
enum AutoSpliceIn {
    static let minConfirmations = 3
    static let maxFeeSats: UInt64 = 5_000
    static let maxFeePercent: UInt64 = 50
    /// Splice-in size estimate: shared input and output plus one wallet input.
    static let estimatedVBytes: UInt64 = 170
    static let retryAfter: TimeInterval = 10 * 60

    enum Step: Equatable {
        case idle
        case confirming
        case retryLater
        case feeTooHigh
        case splice
    }

    /// The closures are only called once their check is reached; the fee rate is a network call.
    static func decide(
        canSplice: Bool,
        spendableSats: UInt64,
        newestConfirmations: () async -> Int?,
        lastAttemptAt: Date?,
        now: Date,
        feeRateSatVb: () async -> UInt64?
    ) async -> Step {
        guard canSplice, spendableSats > 0 else { return .idle }
        // nil means no confirmed wallet transaction is known, which does not block.
        if let confirmations = await newestConfirmations(), confirmations < minConfirmations {
            return .confirming
        }
        // One automatic attempt per retryAfter, so a failing splice is not retried every tick.
        if let lastAttemptAt, now.timeIntervalSince(lastAttemptAt) < retryAfter {
            return .retryLater
        }
        guard let feeRate = await feeRateSatVb() else { return .retryLater }
        let (fee, overflow) = feeRate.multipliedReportingOverflow(by: estimatedVBytes)
        if overflow || fee > maxFeeSats || fee * 100 > spendableSats * maxFeePercent {
            return .feeTooHigh
        }
        return .splice
    }

    /// Confirmations of the newest confirmed transaction, or nil when there is none.
    static func newestConfirmations(confirmedHeights: [Int], tipHeight: Int) -> Int? {
        guard let newest = confirmedHeights.max() else { return nil }
        return max(0, tipHeight - newest + 1)
    }
}
