import Foundation
import os.log

@MainActor
final class ConfirmationPollingService {
    private let databaseService: DatabaseService
    private let blockHeightService: BlockHeightService
    private let confirmationService: ConfirmationService
    private let pollInterval: TimeInterval
    private let logger = Logger(subsystem: "com.stablechannels", category: "confirmation")

    private var pollTask: Task<Void, Never>?

    /// Fires after each poll cycle. Observers should re-load their
    /// payment list to reflect updated confirmation state.
    var onUpdate: (@MainActor () -> Void)?

    init(
        databaseService: DatabaseService,
        blockHeightService: BlockHeightService,
        confirmationService: ConfirmationService,
        pollInterval: TimeInterval = 30
    ) {
        self.databaseService = databaseService
        self.blockHeightService = blockHeightService
        self.confirmationService = confirmationService
        self.pollInterval = pollInterval
    }

    func start() {
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.pollOnce()
                try? await Task.sleep(nanoseconds: UInt64((self?.pollInterval ?? 30) * 1_000_000_000))
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
    }

    func pollOnce() async {
        let currentHeight = blockHeightService.currentHeight
        guard currentHeight > 0 else { return }

        let pending: [PaymentRecord]
        do {
            pending = try databaseService.paymentsNeedingConfirmation(currentBlockHeight: currentHeight)
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
