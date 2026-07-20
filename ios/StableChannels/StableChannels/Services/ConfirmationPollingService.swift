import Foundation
import os.log

@MainActor
final class ConfirmationPollingService {
    private let databaseService: DatabaseService
    private let blockHeightService: BlockHeightService
    private let confirmationService: ConfirmationService
    private let logger = Logger(subsystem: "com.stablechannels", category: "confirmation")

    /// True while a poll cycle is in progress, prevents concurrent runs.
    private var isPolling = false

    /// Fires after each poll cycle. Observers should re-load their
    /// payment list to reflect updated confirmation state.
    var onUpdate: (@MainActor () -> Void)?

    init(
        databaseService: DatabaseService,
        blockHeightService: BlockHeightService,
        confirmationService: ConfirmationService
    ) {
        self.databaseService = databaseService
        self.blockHeightService = blockHeightService
        self.confirmationService = confirmationService
    }

    /// Called by BlockHeightService whenever the chain tip changes.
    /// Also safe to call manually for an initial sync on app launch.
    func pollOnce() async {
        guard !isPolling else { return }
        isPolling = true
        defer { isPolling = false }

        let currentHeight = blockHeightService.currentHeight
        guard currentHeight > 0 else { return }

        let pending: [PaymentRecord]
        do {
            pending = try databaseService.paymentsNeedingConfirmation()
        } catch {
            logger.error("Failed to load pending confirmations: \(error.localizedDescription)")
            return
        }

        for payment in pending {
            guard !Task.isCancelled else { return }
            await resolve(payment: payment, currentHeight: currentHeight)
        }

        onUpdate?()
    }

    private func resolve(payment: PaymentRecord, currentHeight: UInt32) async {
        let outcome = await confirmationService.resolve(
            payment: payment,
            currentBlockHeight: currentHeight
        )
        switch outcome {
        case .confirmed(let progress, let blockHeight):
            // Skip redundant writes — only update if confirmations actually changed
            guard progress.display != payment.confirmations else { return }
            do {
                try databaseService.updateConfirmations(
                    paymentId: payment.id,
                    txBlockHeight: blockHeight,
                    currentBlockHeight: currentHeight
                )
                AuditService.log("CONFIRMATION_UPDATE", data: [
                    "payment_id": "\(payment.id)",
                    "confirmations": "\(progress.display)",
                    "block_height": "\(blockHeight)"
                ])
            } catch {
                logger.error("Failed to update confirmations: \(error.localizedDescription)")
            }
        case .error(let message):
            AuditService.log("CONFIRMATION_RESOLVE_FAILED", data: [
                "payment_id": "\(payment.id)",
                "error": message
            ])
        case .pending, .noTxid:
            break
        }
    }
}
