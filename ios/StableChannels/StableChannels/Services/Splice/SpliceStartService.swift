import Foundation

/// The pending history row a splice start saves before the native call.
struct SpliceStartRequest {
    let paymentType: SplicePaymentType
    let amountSats: UInt64
    let price: Double
    var address: String?
    /// Whether the destination is our own address, saved while that is still provable.
    var selfSend = false
    let channel: SpliceChannelIdentity
}

/// The one start sequence for splice-out sends and sweeps into the channel: the row is saved
/// before the node is asked, so every later event has an operation to match, and settled once the
/// node answers. AppState holds the lock state around it.
@MainActor
struct SpliceStartService {
    let spliceRepo: SpliceRepository

    /// Saves the pending row and the operation identity; returns the row id.
    func record(_ request: SpliceStartRequest) throws -> Int64 {
        guard let amountMsat = Int64(exactly: request.amountSats).flatMap(OnchainAmount.msats(fromSats:)) else {
            throw DatabaseError.executeFailed("Splice amount out of range")
        }
        return try spliceRepo.recordPendingSplice(
            paymentType: request.paymentType.rawValue,
            amountMsat: amountMsat,
            amountUSD: OnchainAmount.usd(sats: Int64(request.amountSats), price: request.price),
            btcPrice: request.price > 0 ? request.price : nil,
            address: request.address,
            selfSend: request.selfSend,
            channel: request.channel
        )
    }

    /// The node accepted the splice saved as [rowId]; older overdue attempts on its channel are dead.
    func accepted(rowId: Int64) {
        do {
            try spliceRepo.retireSupersededSplices(byRowId: rowId)
        } catch {
            AuditService.log("SPLICE_RETIRE_FAILED", data: ["error": error.localizedDescription])
        }
    }

    /// The native call failed: the saved row [rowId] (the latest one when unknown) is terminal.
    @discardableResult
    func failed(rowId: Int64?) -> Bool {
        spliceRepo.failPendingSplice(rowId: rowId)
    }
}
