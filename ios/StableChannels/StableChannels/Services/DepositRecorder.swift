import Foundation

/// Records a detected on-chain deposit. Two strategies:
///
/// - `KnownAddress`: insert a resolution row, write a payment row linked
///   via `resolutionId`, and kick the Esplora resolver.
/// - `UnknownAddress`: write a completed payment row with no resolver
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
    /// leave `prevOnchainSats` untouched.
    @discardableResult
    func record(deposit: DepositRecordInput, address: String?) -> Bool
}

/// Ties a balance increase to the receives the WebSocket already recorded, by txid, and corrects
/// rows whose feed amount differs from the node's. The rules are in ``DepositAttribution``.
struct OnchainDepositReconciler {
    let repository: OnchainReceiveRepository

    /// The part of [depositSats] no recorded receive explains, or nil when the recorded receives
    /// could not be read.
    /// Known limitation: an unknown [baseline] (first launch after upgrade) counts nothing as
    /// arrived, so a WebSocket receive in that one increase can still be recorded twice.
    func remainderSats(depositSats: Int64, wallet: WalletInboundOnchain, baseline: Set<String>?) -> Int64? {
        let arrived = DepositAttribution.arrivedTxids(wallet: wallet.txids, baseline: baseline ?? wallet.txids)
        guard let recorded = try? repository.recordedReceiveSats(txids: arrived) else { return nil }
        let attribution = DepositAttribution.attribute(
            depositSats: depositSats,
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
        return attribution.remainderSats
    }
}

struct DepositRecordInput {
    let depositId: String
    let depositSats: Int64
    let amountUSD: Double?
    let btcPrice: Double?
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
        onLaunchResolver(resolutionId, address)
        return true
    }
}

/// Records when no current receive address is known. Writes a completed
/// payment row with no resolver; user can re-generate an address to see
/// the on-chain link. Returns `false` if the write fails so the caller
/// can leave `prevOnchainSats` unchanged and retry on the next balance
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
            return true
        } catch {
            return false
        }
    }
}
