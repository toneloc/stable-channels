import Foundation
import SwiftUI

@Observable
@MainActor
final class ConfirmationService {
    private let provider: TxConfirmationProvider
    private let calculator: ConfirmationCalculating

    init(
        provider: TxConfirmationProvider,
        calculator: ConfirmationCalculating = ConfirmationCalculator()
    ) {
        self.provider = provider
        self.calculator = calculator
    }

    func resolve(
        payment: PaymentRecord,
        currentBlockHeight: UInt32,
        forceRecheck: Bool = false
    ) async -> ConfirmationOutcome {
        guard let txid = payment.txid, !txid.isEmpty else {
            return .noTxid
        }
        let required = ConfirmationPolicy.requiredConfirmations(for: payment.paymentType)
        // Fast path: only trust cached tx_block_height when not forceRechecking
        if !forceRecheck, let existingHeight = payment.txBlockHeight, existingHeight > 0 {
            let progress = calculator.progress(
                for: existingHeight,
                currentBlockHeight: currentBlockHeight,
                required: required
            )
            if progress.isComplete {
                return .confirmed(progress: progress, blockHeight: existingHeight)
            }
        }
        do {
            let lookup: EsploraLookup<TxStatus>
            if let address = ConfirmationPolicy.expectedReceiveAddress(for: payment) {
                switch try await provider.receiveCheck(for: txid, paysTo: address) {
                case .found(let check):
                    guard check.paysToAddress else { return .addressMismatch }
                    if let status = check.status {
                        lookup = .found(status)
                    } else {
                        lookup = try await provider.status(for: txid)
                    }
                case .notFound:
                    lookup = .notFound
                case .unavailable:
                    lookup = .unavailable
                }
            } else {
                lookup = try await provider.status(for: txid)
            }
            switch lookup {
            case .found(let status):
                guard let height = status.blockHeight else { return .pending }
                let progress = calculator.progress(
                    for: height,
                    currentBlockHeight: currentBlockHeight,
                    required: required
                )
                return .confirmed(progress: progress, blockHeight: height)
            case .notFound:
                return .notFound
            case .unavailable:
                return .error(EsploraError.invalidResponse.localizedDescription)
            }
        } catch {
            return .error(error.localizedDescription)
        }
    }
}

enum ConfirmationOutcome: Equatable {
    case noTxid
    case pending
    case confirmed(progress: ConfirmationProgress, blockHeight: UInt32)
    /// Every explorer answered 404 for the txid.
    case notFound
    /// A received onchain row's txid does not pay the row's stored address.
    case addressMismatch
    case error(String)
}
