import Foundation
import os.log

/// Outcome of a manual confirmation refresh (History pull-to-refresh).
enum ConfirmationRefreshResult: Equatable {
    case completed(failedLookups: Int)
    case chainTipUnavailable
    case databaseUnavailable

    /// User-facing error, or nil when every lookup succeeded.
    var errorMessage: String? {
        switch self {
        case .completed(let failedLookups):
            switch failedLookups {
            case 0:
                return nil
            case 1:
                return String(
                    localized: "history_refresh_failed_one",
                    defaultValue: "Couldn't check 1 transaction. Pull to try again."
                )
            default:
                return String(
                    localized: "history_refresh_failed_many",
                    defaultValue: "Couldn't check \(failedLookups) transactions. Pull to try again."
                )
            }
        case .chainTipUnavailable:
            return String(
                localized: "history_refresh_chain_unavailable",
                defaultValue: "Couldn't reach the block explorer. Pull to try again."
            )
        case .databaseUnavailable:
            return String(
                localized: "history_refresh_db_unavailable",
                defaultValue: "Payment history is unavailable right now."
            )
        }
    }
}

@MainActor
final class ConfirmationPollingService {
    private let databaseService: DatabaseService
    private let blockHeightService: BlockHeightService
    private let confirmationService: ConfirmationService
    /// Single-attempt lookups for manual refresh, so a user-initiated pull fails fast instead of
    /// sitting through the automatic poller's retry backoff (which can take minutes offline).
    private let manualTipProvider: BlockHeightProvider?
    private let manualConfirmationService: ConfirmationService?
    private let logger = Logger(subsystem: "com.stablechannels", category: "confirmation")

    /// True while a poll cycle is in progress, prevents concurrent runs.
    private var isPolling = false
    /// Manual refreshes waiting for the in-flight cycle to finish.
    private var idleWaiters: [CheckedContinuation<Void, Never>] = []

    /// Fires after each poll cycle. Observers should re-load their
    /// payment list to reflect updated confirmation state.
    var onUpdate: (@MainActor () -> Void)?

    init(
        databaseService: DatabaseService,
        blockHeightService: BlockHeightService,
        confirmationService: ConfirmationService,
        manualTipProvider: BlockHeightProvider? = nil,
        manualConfirmationService: ConfirmationService? = nil
    ) {
        self.databaseService = databaseService
        self.blockHeightService = blockHeightService
        self.confirmationService = confirmationService
        self.manualTipProvider = manualTipProvider
        self.manualConfirmationService = manualConfirmationService
    }

    /// Called by BlockHeightService whenever the chain tip changes.
    /// Also safe to call manually for an initial sync on app launch.
    func pollOnce() async {
        guard !isPolling else { return }
        isPolling = true
        defer { finishPass() }

        let currentHeight = blockHeightService.currentHeight
        guard currentHeight > 0 else { return }

        let pending: [PaymentRecord]
        do {
            pending = try databaseService.paymentRepo.paymentsNeedingConfirmation()
        } catch {
            logger.error("Failed to load pending confirmations: \(error.localizedDescription)")
            return
        }

        var anyUpdated = false
        for payment in pending {
            guard !Task.isCancelled else { return }
            if await resolve(payment: payment, currentHeight: currentHeight) == .updated {
                anyUpdated = true
            }
        }

        if anyUpdated {
            onUpdate?()
        }
    }

    /// Manual refresh: waits for any in-flight cycle (instead of skipping), fetches a fresh
    /// chain tip and re-resolves pending payments. Reports a failed tip fetch or failed
    /// transaction lookups rather than treating them as success.
    func refresh() async throws -> ConfirmationRefreshResult {
        while isPolling {
            await withCheckedContinuation { idleWaiters.append($0) }
        }
        try Task.checkCancellation()
        isPolling = true
        defer { finishPass() }

        let currentHeight: UInt32
        if let manualTipProvider {
            guard let height = try? await manualTipProvider.currentHeight(), height > 0 else {
                try Task.checkCancellation()
                return .chainTipUnavailable
            }
            // This pass resolves pending payments itself; don't trigger a second automatic poll.
            blockHeightService.setHeightSilently(height)
            currentHeight = height
        } else {
            guard let height = await blockHeightService.refresh(), height > 0 else {
                try Task.checkCancellation()
                return .chainTipUnavailable
            }
            currentHeight = height
        }

        let pending: [PaymentRecord]
        do {
            pending = try databaseService.paymentRepo.paymentsNeedingConfirmation()
        } catch {
            logger.error("Failed to load pending confirmations: \(error.localizedDescription)")
            return .databaseUnavailable
        }

        var anyUpdated = false
        var failedLookups = 0
        for payment in pending {
            if Task.isCancelled { break }
            switch await resolve(
                payment: payment,
                currentHeight: currentHeight,
                using: manualConfirmationService ?? confirmationService
            ) {
            case .updated: anyUpdated = true
            case .failed: failedLookups += 1
            case .unchanged: break
            }
        }

        if anyUpdated {
            onUpdate?()
        }
        try Task.checkCancellation()
        return .completed(failedLookups: failedLookups)
    }

    private func finishPass() {
        isPolling = false
        let waiters = idleWaiters
        idleWaiters.removeAll()
        waiters.forEach { $0.resume() }
    }

    /// Revalidates both pending payments and recently completed payments (last ~12 blocks)
    /// against Esplora. Triggered during an offline gap or reorg event.
    func revalidateRecentPayments(windowDepth: UInt32 = 12) async {
        guard !isPolling else { return }
        isPolling = true
        defer { finishPass() }

        // Refresh authoritative chain tip from Esplora first
        await blockHeightService.refresh()
        let currentHeight = blockHeightService.currentHeight
        guard currentHeight > 0 else { return }

        var anyUpdated = false

        // 1. Process pending payments
        if let pending = try? databaseService.paymentRepo.paymentsNeedingConfirmation() {
            for payment in pending {
                guard !Task.isCancelled else { return }
                if await resolve(payment: payment, currentHeight: currentHeight) == .updated {
                    anyUpdated = true
                }
            }
        }

        // 2. Revalidate recently confirmed payments (last ~12 blocks)
        let windowStart = currentHeight >= windowDepth ? currentHeight - windowDepth : 0
        if let recentConfirmed = try? databaseService.paymentRepo
            .recentConfirmedPayments(confirmedAfterHeight: windowStart) {
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
                        try databaseService.paymentRepo.downgradePaymentToPending(paymentId: payment.id)
                        anyUpdated = true
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
                        do {
                            try databaseService.paymentRepo.updateConfirmations(
                                paymentId: payment.id,
                                paymentType: payment.paymentType,
                                txBlockHeight: blockHeight,
                                currentBlockHeight: currentHeight
                            )
                            anyUpdated = true
                        } catch {
                            logger.error("Failed to update confirmations: \(error.localizedDescription)")
                        }
                    }
                case .error, .noTxid:
                    break
                }
            }
        }

        if anyUpdated {
            onUpdate?()
        }
    }

    private enum ResolveResult {
        case updated
        case unchanged
        case failed
    }

    private func resolve(
        payment: PaymentRecord,
        currentHeight: UInt32,
        using service: ConfirmationService? = nil
    ) async -> ResolveResult {
        let outcome = await (service ?? confirmationService).resolve(
            payment: payment,
            currentBlockHeight: currentHeight
        )
        switch outcome {
        case .confirmed(let progress, let blockHeight):
            // Skip redundant writes — only update if confirmations actually changed OR if block height changed (reorg)
            guard progress.display != payment.confirmations || blockHeight != payment.txBlockHeight
            else { return .unchanged }
            do {
                try databaseService.paymentRepo.updateConfirmations(
                    paymentId: payment.id,
                    paymentType: payment.paymentType,
                    txBlockHeight: blockHeight,
                    currentBlockHeight: currentHeight
                )
                AuditService.log("CONFIRMATION_UPDATE", data: [
                    "payment_id": "\(payment.id)",
                    "confirmations": "\(progress.display)",
                    "block_height": "\(blockHeight)"
                ])
                return .updated
            } catch {
                logger.error("Failed to update confirmations: \(error.localizedDescription)")
                return .failed
            }
        case .error(let message):
            AuditService.log("CONFIRMATION_RESOLVE_FAILED", data: [
                "payment_id": "\(payment.id)",
                "error": message
            ])
            return .failed
        case .pending, .noTxid:
            return .unchanged
        }
    }
}
