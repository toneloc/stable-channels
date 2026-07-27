import Foundation
import SwiftUI

@Observable
@MainActor
final class ConfirmationService {
    private let provider: TxConfirmationProvider
    private let calculator = ConfirmationCalculator()

    init(provider: TxConfirmationProvider) {
        self.provider = provider
    }

    func resolve(payment: PaymentRecord, currentBlockHeight: UInt32) async -> ConfirmationOutcome {
        guard let txid = payment.txid, !txid.isEmpty else {
            return .noTxid
        }
        // Fast path: only trust the cached tx_block_height for payments that
        // have already reached full confirmation (6+ confs). These are deep
        // enough that a reorg is essentially impossible.
        //
        // For payments still under 6 confs, always re-fetch from Esplora so
        // shallow reorgs that move the tx to a different block are caught
        // immediately. Cost: one cheap call per pending payment per block.
        if let existingHeight = payment.txBlockHeight, existingHeight > 0 {
            let progress = calculator.progress(for: existingHeight, currentBlockHeight: currentBlockHeight)
            if progress.isComplete {
                return .confirmed(progress: progress, blockHeight: existingHeight)
            }
        }
        do {
            guard let height = try await provider.blockHeight(for: txid) else {
                return .pending
            }
            let progress = calculator.progress(for: height, currentBlockHeight: currentBlockHeight)
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
