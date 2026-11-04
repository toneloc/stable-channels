import Foundation

/// Pure decisions taken around a splice's in-memory lock and its completion.
enum SpliceCompletionPolicy {
    /// What the status line says once a channel reports ready.
    enum ChannelReadyStatus: Equatable {
        case movePendingConfirmation
        case channelReady
    }

    /// New baseline for the balance-delta deposit detector once a splice row completes, or nil to
    /// leave it unchanged. A splice-out paying one of our own addresses (a self-send) raises the
    /// on-chain balance by its own settled amount; advancing by exactly that amount absorbs the
    /// splice's own effect without swallowing a separate deposit that landed in the same window.
    /// Capped at the current balance (network fees can make the actual rise slightly smaller).
    /// An external destination never raises our balance, so it never moves the baseline.
    static func advancedOnchainBaseline(
        baselineSats: UInt64,
        currentOnchainSats: UInt64,
        completedRow: PaymentRecord,
        destinationIsOwnAddress: Bool
    ) -> UInt64? {
        guard completedRow.paymentType == SplicePaymentType.spliceOut.rawValue,
              completedRow.direction == SplicePaymentType.spliceOut.direction,
              destinationIsOwnAddress else { return nil }
        let (sum, overflow) = baselineSats.addingReportingOverflow(completedRow.amountSats)
        return overflow ? currentOnchainSats : min(sum, currentOnchainSats)
    }

    /// Status line once a channel reports ready: a move is still confirming while its lock and
    /// negotiated txid are both held.
    static func channelReadyStatus(isSweeping: Bool, hasSpliceTxid: Bool) -> ChannelReadyStatus {
        isSweeping && hasSpliceTxid ? .movePendingConfirmation : .channelReady
    }

    /// A lock restored for a pre-negotiation row outlives that row's expiry only in memory; drop
    /// it once no pending splice row remains. `hasPendingSpliceRow` is nil when the check failed,
    /// which must read as "still pending".
    static func shouldReleaseStaleLock(
        isSweeping: Bool,
        hasSpliceTxid: Bool,
        isMonitoringConfirmation: Bool,
        hasPendingSpliceRow: Bool?
    ) -> Bool {
        guard isSweeping, !hasSpliceTxid, !isMonitoringConfirmation else { return false }
        return hasPendingSpliceRow == false
    }
}
