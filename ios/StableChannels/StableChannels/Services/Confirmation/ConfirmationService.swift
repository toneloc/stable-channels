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
            guard let height = try await provider.blockHeight(for: txid) else {
                return .pending
            }
            let progress = calculator.progress(for: height, currentBlockHeight: currentBlockHeight, required: required)
            return .confirmed(progress: progress, blockHeight: height)
        } catch {
            return .error(error.localizedDescription)
        }
    }
}

enum ConfirmationOutcome: Equatable {
    case noTxid
    case pending
    case confirmed(progress: ConfirmationProgress, blockHeight: UInt32)
    case error(String)
}
