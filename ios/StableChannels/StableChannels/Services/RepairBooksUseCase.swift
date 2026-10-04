import Foundation

/// Orchestrates policy validation and execution for repairing overbacked stable books.
final class RepairBooksUseCase {
    private let paymentStatusProvider: PaymentStatusProviding
    private let spliceStatusProvider: SpliceStatusProviding
    private let stabilitySendStatusProvider: StabilitySendStatusProviding

    struct Context {
        let hasUserChannelId: Bool
        let hasReadyChannel: Bool
        let isChannelClosing: Bool
        let isSweeping: Bool
        let hasPendingSpliceInMemory: Bool
        let price: Double
    }

    init(
        paymentStatusProvider: PaymentStatusProviding,
        spliceStatusProvider: SpliceStatusProviding,
        stabilitySendStatusProvider: StabilitySendStatusProviding
    ) {
        self.paymentStatusProvider = paymentStatusProvider
        self.spliceStatusProvider = spliceStatusProvider
        self.stabilitySendStatusProvider = stabilitySendStatusProvider
    }

    /// Evaluates repair preconditions and heals overbacked channel books if safe.
    /// Employs lazy short-circuiting to avoid disk and node lookups when in-memory invariants fail.
    @discardableResult
    func execute(
        channel: inout StableChannel,
        context: Context
    ) -> StabilityService.RepairResult? {
        // Early exit: skip checks if books are not overbacked
        guard channel.backingSats > channel.stableReceiverBTC.sats else {
            return nil
        }

        // Early exit: skip queries if channel is unready or closing
        guard BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: context.hasUserChannelId,
            hasReadyChannel: context.hasReadyChannel,
            isChannelClosing: context.isChannelClosing,
            isSweeping: context.isSweeping,
            hasPendingSpliceInMemory: context.hasPendingSpliceInMemory,
            price: context.price
        ) else {
            return nil
        }

        // Lazy I/O checks: short-circuit at the first pending operation
        guard !spliceStatusProvider.hasPendingSplice() else {
            return nil
        }

        guard !stabilitySendStatusProvider.hasPendingStabilitySend() else {
            return nil
        }

        guard !paymentStatusProvider.hasPendingOutgoingPayments() else {
            return nil
        }

        return StabilityService.repairBooksAboveLiveBalance(&channel, price: context.price)
    }
}
