import Foundation

/// The database side of completing a confirmed splice: books first, then the self-send baseline,
/// then the row. AppState runs the steps in order and keeps only its in-memory state.
@MainActor
struct SpliceCompletionService {
    let databaseService: DatabaseService

    /// A captured event whose txid assignment collided (e.g. with a self-send deposit row) left the
    /// row's txid NULL, and completion needs an exact match. Retries from captured events only; a
    /// no-op once a splice row carries the txid.
    func recoverAssignments() {
        try? databaseService.spliceRepo.recoverAssignments()
    }

    /// Reconciles the stable books against the stored row, before the row completes, so a signed
    /// correction or background settlement is never overwritten by a stale snapshot. Adopts the
    /// stored books into [channel]; returns the repair the splice forced, if any.
    func reconcileBooks(_ channel: inout StableChannel, price: Double) throws -> StabilityService.RepairResult? {
        let books = try databaseService.channelRepo.reconcileOutgoingBacking(
            userChannelId: channel.userChannelId,
            receiverSats: channel.stableReceiverBTC.sats,
            price: price
        )
        StabilityService.adoptStoredBooks(&channel, expectedUSD: books.expectedUSD, backingSats: books.backingSats)
        return books.repair
    }

    /// The deposit baseline after a self-send splice-out [txid] completes, or nil to leave it: a
    /// splice paying our own address raises the on-chain balance by its own amount. Read from the
    /// splice's row, not in-memory splice state that may belong to a newer operation. The flag
    /// saved at the start wins over a fresh check: by now a receive recorded on the splice's own
    /// output can make the destination look like one of ours.
    func selfSendBaseline(
        txid: String,
        baseline: OnchainDepositBaseline,
        onchainSats: UInt64,
        currentReceiveAddress: String?
    ) -> UInt64? {
        guard !baseline.counts(txid), let row = databaseService.paymentRepo.payment(txid: txid) else { return nil }
        let isOwnAddress = (try? databaseService.spliceRepo.isSelfSendSplice(txid: txid))
            ?? databaseService.onchainRepo.isOwnReceiveAddress(
                row.address,
                currentReceiveAddress: currentReceiveAddress
            )
        return SpliceCompletionPolicy.advancedOnchainBaseline(
            baselineSats: baseline.sats,
            currentOnchainSats: onchainSats,
            completedRow: row,
            destinationIsOwnAddress: isOwnAddress
        )
    }

    /// Marks the splice row completed; false when no row moved, so the caller retries.
    func completeRow(txid: String) -> Bool {
        databaseService.spliceRepo.completeSplice(txid: txid)
    }
}
