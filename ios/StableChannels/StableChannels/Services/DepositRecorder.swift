import Foundation

/// Records a detected on-chain deposit. Two strategies:
///
/// - `KnownAddress`: insert a resolution row, write a payment row linked
///   via `resolutionId`, and kick the Esplora resolver.
/// - `UnknownAddress`: write a pending payment row with no resolver
///   (the explorer link is unavailable until the user re-generates an
///   address).
///
/// Each strategy handles its own crash-safety ordering. The protocol
/// keeps AppState's `detectOnchainDeposit` branch as a single dispatch
/// instead of a hard-coded `if let address` ladder.
@MainActor
protocol DepositRecorder {
    /// Returns true if the deposit was recorded (and resolver launched if
    /// applicable). False means a precondition failed and the caller should
    /// leave the deposit baseline untouched.
    @discardableResult
    func record(deposit: DepositRecordInput, address: String?) -> Bool
}

/// Ties a balance increase to the receives the WebSocket already recorded, by txid, and corrects
/// rows whose feed amount differs from the node's. The rules are in ``DepositAttribution``.
struct OnchainDepositReconciler {
    /// Part of a balance increase no recorded receive explains, with its txid when the node shows
    /// which transaction it is.
    struct MissedDeposit: Equatable {
        let sats: Int64
        let txid: String?
        /// The arrived txids the deposit can be when [txid] is unknown.
        let candidateTxids: Set<String>
    }

    let repository: OnchainReceiveRepository

    /// The part of [depositSats] no recorded receive explains, or nil when the recorded receives
    /// could not be read.
    func missedDeposit(
        depositSats: Int64,
        wallet: WalletInboundOnchain,
        baseline: OnchainDepositBaseline
    ) -> MissedDeposit? {
        let arrived: Set<String>
        let recorded: [String: Int64]
        do {
            if let baselineTxids = baseline.txids {
                arrived = DepositAttribution.arrivedTxids(wallet: wallet.txids, baseline: baselineTxids)
                recorded = try repository.recordedReceiveSats(txids: arrived)
            } else {
                // Unknown baseline txids (first launch after upgrade): a receive recorded since tracking
                // began is not in the cached balance. Known limitation: one recorded before it that the
                // cached balance did not count yet is still recorded twice.
                recorded = try repository.recordedReceiveSats(txids: wallet.txids, recordedSince: baseline.since)
                arrived = Set(recorded.keys)
            }
        } catch {
            return nil
        }
        let attribution = DepositAttribution.attribute(
            depositSats: depositSats,
            arrived: arrived,
            recorded: recorded,
            walletSats: wallet.sats
        )
        for (txid, sats) in attribution.amountCorrections {
            do {
                try repository.correctReceiveAmount(txid: txid, sats: sats)
                AuditService.log("ONCHAIN_RECEIVE_AMOUNT_CORRECTED", data: ["txid": txid, "sats": "\(sats)"])
            } catch {
                AuditService.log("ONCHAIN_RECEIVE_AMOUNT_CORRECT_FAILED", data: ["error": "\(error)"])
            }
        }
        if attribution.matchedRecordedReceive {
            AuditService.log("ONCHAIN_DEPOSIT_MATCHED_WEBSOCKET", data: [
                "amount_sats": "\(depositSats)",
                "remainder_sats": "\(attribution.remainderSats)"
            ])
        }
        return MissedDeposit(
            sats: attribution.remainderSats,
            txid: attribution.missedTxid,
            candidateTxids: attribution.unrecordedTxids
        )
    }
}

struct DepositRecordInput {
    let depositId: String
    let depositSats: Int64
    let amountUSD: Double?
    let btcPrice: Double?
    /// The arrived txids this deposit can be. A late WebSocket receive takes the row over only for
    /// one of these; a row without any is never taken over.
    var candidateTxids: Set<String> = []
}

/// Stores a fallback row's candidate txids after the row. A crash or failure in between leaves a row
/// without candidates, which a WebSocket receive never takes over: it records its own row instead.
@MainActor
private func recordCandidates(_ deposit: DepositRecordInput, databaseService: DatabaseService) {
    guard !deposit.candidateTxids.isEmpty else { return }
    do {
        try databaseService.onchainRepo.recordDepositCandidates(
            paymentId: deposit.depositId,
            txids: deposit.candidateTxids
        )
    } catch {
        AuditService.log("ONCHAIN_DEPOSIT_CANDIDATES_FAILED", data: ["error": "\(error)"])
    }
}

/// Records when the current receive address is known. Crash-safe ordering:
/// resolver row first, payment row linked via `resolution_id`. Crash
/// between the two leaves an orphan resolver row (harmless on replay).
@MainActor
final class KnownAddressDepositRecorder: DepositRecorder {
    private let databaseService: DatabaseService?
    private let onLaunchResolver: (Int64, String) -> Void

    init(databaseService: DatabaseService?, onLaunchResolver: @escaping (Int64, String) -> Void) {
        self.databaseService = databaseService
        self.onLaunchResolver = onLaunchResolver
    }

    func record(deposit: DepositRecordInput, address: String?) -> Bool {
        guard let address, !address.isEmpty else { return false }
        guard let resolutionId = databaseService?.onchainRepo.insertOnchainReceiveResolution(address: address) else {
            AuditService.log("ONCHAIN_RECEIVE_RES_INSERT_FAILED", data: ["address": address])
            return false
        }
        let ok = databaseService?.onchainRepo.recordOnchainPaymentWithResolution(
            paymentId: deposit.depositId,
            amountMsat: Int64(deposit.depositSats) * 1000,
            amountUSD: deposit.amountUSD,
            btcPrice: deposit.btcPrice,
            resolutionId: resolutionId
        ) ?? false
        if !ok {
            _ = databaseService?.onchainRepo.deleteOnchainReceiveResolution(id: resolutionId)
            return false
        }
        if let databaseService { recordCandidates(deposit, databaseService: databaseService) }
        onLaunchResolver(resolutionId, address)
        return true
    }
}

/// Records when no current receive address is known. Writes a pending
/// payment row with no resolver; user can re-generate an address to see
/// the on-chain link. Returns `false` if the write fails so the caller
/// can leave the deposit baseline unchanged and retry on the next balance
/// poll (matches `KnownAddressDepositRecorder` semantics).
@MainActor
final class UnknownAddressDepositRecorder: DepositRecorder {
    private let databaseService: DatabaseService?

    init(databaseService: DatabaseService?) {
        self.databaseService = databaseService
    }

    func record(deposit: DepositRecordInput, address _: String?) -> Bool {
        guard let databaseService else { return false }
        do {
            try databaseService.paymentRepo.recordPayment(
                paymentId: deposit.depositId,
                paymentType: "onchain",
                direction: "received",
                amountMsat: UInt64(Int64(deposit.depositSats) * 1000),
                amountUSD: deposit.amountUSD,
                btcPrice: deposit.btcPrice,
                counterparty: nil,
                status: "pending"
            )
            recordCandidates(deposit, databaseService: databaseService)
            return true
        } catch {
            return false
        }
    }
}

/// Records a deposit the node identified by txid, as the WebSocket would have, so a late WebSocket
/// event for it finds the row instead of taking over another deposit's placeholder. Falls back to
/// [fallback] when a row for the txid already exists (one the feed reported removed).
@MainActor
final class IdentifiedDepositRecorder: DepositRecorder {
    private let txid: String
    private let databaseService: DatabaseService?
    private let fallback: DepositRecorder

    init(txid: String, databaseService: DatabaseService?, fallback: DepositRecorder) {
        self.txid = txid
        self.databaseService = databaseService
        self.fallback = fallback
    }

    func record(deposit: DepositRecordInput, address: String?) -> Bool {
        guard let databaseService else { return false }
        do {
            let inserted = try databaseService.paymentRepo.recordPayment(
                paymentId: OnchainReceiveRepository.receivePaymentId(txid: txid),
                paymentType: "onchain",
                direction: "received",
                amountMsat: UInt64(deposit.depositSats) * 1000,
                amountUSD: deposit.amountUSD,
                btcPrice: deposit.btcPrice,
                counterparty: nil,
                status: "pending",
                txid: txid,
                address: address
            )
            return inserted || fallback.record(deposit: deposit, address: address)
        } catch {
            return false
        }
    }
}
