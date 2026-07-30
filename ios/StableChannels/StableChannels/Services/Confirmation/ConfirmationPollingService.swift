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
            pending = try databaseService.paymentRepo.paymentsNeedingConfirmation()
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

    /// Revalidates both pending payments and recently completed payments (last ~12 blocks)
    /// against Esplora. Triggered during an offline gap or reorg event.
    func revalidateRecentPayments(windowDepth: UInt32 = 12) async {
        guard !isPolling else { return }
        isPolling = true
        defer { isPolling = false }

        // Refresh authoritative chain tip from Esplora first
        await blockHeightService.refresh()
        let currentHeight = blockHeightService.currentHeight
        guard currentHeight > 0 else { return }

        // 1. Process pending payments
        if let pending = try? databaseService.paymentsNeedingConfirmation() {
            for payment in pending {
                guard !Task.isCancelled else { return }
                await resolve(payment: payment, currentHeight: currentHeight)
            }
        }

        // 2. Revalidate recently confirmed payments (last ~12 blocks)
        let windowStart = currentHeight >= windowDepth ? currentHeight - windowDepth : 0
        if let recentConfirmed = try? databaseService.recentConfirmedPayments(confirmedAfterHeight: windowStart) {
            for payment in recentConfirmed {
                guard !Task.isCancelled else { return }
                let outcome = await confirmationService.resolve(
                    payment: payment,
                    currentBlockHeight: currentHeight,
                    forceRecheck: true
                )
                switch outcome {
                case .pending:
                    // Esplora reports transaction is no longer confirmed — downgrade to pending
                    do {
                        try databaseService.downgradePaymentToPending(paymentId: payment.id)
                        logger
                            .warning(
                                "[Confirmation] Payment #\(payment.id) orphaned in reorg/gap — downgraded to pending."
                            )
                        AuditService.log("PAYMENT_REORG_DOWNGRADED", data: [
                            "payment_id": "\(payment.id)",
                            "txid": payment.txid ?? ""
                        ])
                    } catch {
                        logger.error("Failed to downgrade payment: \(error.localizedDescription)")
                    }
                case .confirmed(let progress, let blockHeight):
                    if blockHeight != payment.txBlockHeight || progress.display != payment.confirmations {
                        try? databaseService.updateConfirmations(
                            paymentId: payment.id,
                            txBlockHeight: blockHeight,
                            currentBlockHeight: currentHeight
                        )
                    }
                case .error, .noTxid:
                    break
                }
            }
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
            // Skip redundant writes — only update if confirmations actually changed OR if block height changed (reorg)
            guard progress.display != payment.confirmations || blockHeight != payment.txBlockHeight else { return }
            do {
                try databaseService.paymentRepo.updateConfirmations(
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
