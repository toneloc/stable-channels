import Foundation

/// Pre-dispatch rules every send must pass, whichever screen dispatches it.
enum SendPreflight {
    struct Input: Equatable {
        let isOnline: Bool
        let isFeeRateReady: Bool
        let amountSats: UInt64
        let totalDebitSats: UInt64
        let availableSats: UInt64
        /// The on-chain wallet send-all spends whatever the wallet holds, so it has no amount or
        /// balance to check; the node rejects an empty wallet.
        let sendsAllFromOnchainWallet: Bool
    }

    enum Decision: Equatable {
        case proceed
        case offline
        case feeRateNotReady
        case invalidAmount
        case exceedsBalance(availableSats: UInt64)
    }

    static func evaluate(_ input: Input) -> Decision {
        guard input.isOnline else { return .offline }
        guard input.isFeeRateReady else { return .feeRateNotReady }
        if input.sendsAllFromOnchainWallet { return .proceed }
        guard input.amountSats > 0 else { return .invalidAmount }
        guard input.totalDebitSats <= input.availableSats, input.availableSats > 0 else {
            return .exceedsBalance(availableSats: input.availableSats)
        }
        return .proceed
    }
}
