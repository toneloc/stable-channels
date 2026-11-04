import Foundation

/// Recovers on-chain receives the live paths missed (#421, #317) and adopts txids for pending
/// placeholders from the wallet's own payment list. Holds only throttle and dedupe state: the
/// rules live in `MissedReceiveRecovery` / `ReceiveTxidAdoption`, the SQL in
/// `OnchainReceiveRepository`.
@MainActor
final class OnchainReceiveRecoveryService {
    // Owned by AppState.
    weak var databaseService: DatabaseService?

    private let transactionLinkService: TransactionLinkService
    private let makeClient: () -> ResilientEsploraClient
    private let makeReceiveChecker: () -> any TxConfirmationProvider
    private let nowSecs: () -> Int64
    private var lastMissedReceiveCheckSecs: Int64 = 0
    private var rejectedTxidAdoptions = Set<ReceiveTxidAdoption.Candidate>()
    private var isResolvingPlaceholderTxids = false

    init(
        transactionLinkService: TransactionLinkService,
        makeClient: @escaping () -> ResilientEsploraClient,
        makeReceiveChecker: @escaping () -> any TxConfirmationProvider,
        nowSecs: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.transactionLinkService = transactionLinkService
        self.makeClient = makeClient
        self.makeReceiveChecker = makeReceiveChecker
        self.nowSecs = nowSecs
    }

    /// Throttled explorer pass over the receive address. Records by txid, so it is safe during a
    /// splice: the repository skips the splice's own txid, and nothing is recorded while a splice
    /// has no txid. [balanceCounts] tells whether the wallet balance already counts a txid, which a
    /// placeholder listing it needs before it can take it.
    func recoverMissedReceive(address: String?, price: Double, balanceCounts: @escaping (String) -> Bool) {
        guard let address, !address.isEmpty, let databaseService else { return }
        let now = nowSecs()
        guard MissedReceiveRecovery.isCheckDue(lastCheckSecs: lastMissedReceiveCheckSecs, nowSecs: now)
        else { return }
        lastMissedReceiveCheckSecs = now
        let client = makeClient()
        Task { @MainActor [weak self] in
            let receives = await client.fetch(
                endpointBuilder: { ["\(ResilientEsploraClient.trimSlash($0))/address/\(address)/txs"] },
                resultParser: { EsploraTransaction.recentReceives(json: $0, address: address, nowSecs: now) }
            )
            guard let self, let receives, self.databaseService === databaseService else { return }
            for receive in receives {
                do {
                    let recorded = try databaseService.onchainRepo.recordMissedReceive(
                        txid: receive.txid,
                        amountSats: receive.sats,
                        amountUSD: OnchainAmount.usd(sats: receive.sats, price: price),
                        btcPrice: OnchainAmount.storedPrice(price),
                        address: address,
                        sinceSecs: now - MissedReceiveRecovery.windowSeconds,
                        balanceCounted: balanceCounts(receive.txid),
                        hasSpliceWithoutTxid: try databaseService.spliceRepo.hasPendingSpliceWithoutTxid()
                    )
                    guard recorded else { continue }
                    // This receive now owns its row, so the pending deposit link points at it.
                    transactionLinkService.setReceiveTxid(receive.txid)
                    AuditService.log("ONCHAIN_RECEIVE_RECOVERED", data: [
                        "txid": receive.txid,
                        "sats": "\(receive.sats)"
                    ])
                } catch {
                    AuditService.log("ONCHAIN_RECEIVE_RECOVERY_FAILED", data: ["error": "\(error)"])
                }
            }
        }
    }

    /// Adopts a wallet-listed candidate txid for a pending placeholder, only once that txid is
    /// proven to pay the placeholder's address. One pass runs at a time; disproven pairs are never
    /// retried. [wallet] is nil while the wallet cannot be read.
    func resolvePlaceholderTxids(wallet: () -> WalletInboundOnchain?) {
        guard !isResolvingPlaceholderTxids, let databaseService else { return }
        let placeholders = databaseService.onchainRepo.placeholdersWithCandidates()
        guard !placeholders.isEmpty, let wallet = wallet(),
              let recorded = try? databaseService.onchainRepo.recordedReceiveTxids() else { return }
        let candidates = ReceiveTxidAdoption.candidates(
            placeholders: placeholders,
            wallet: wallet,
            recordedTxids: recorded
        ).filter { !rejectedTxidAdoptions.contains($0) }
        guard !candidates.isEmpty else { return }
        let rows = Dictionary(uniqueKeysWithValues: placeholders.map { ($0.id, $0) })
        let checker = makeReceiveChecker()
        isResolvingPlaceholderTxids = true
        Task { @MainActor [weak self] in
            defer { self?.isResolvingPlaceholderTxids = false }
            var paying: [ReceiveTxidAdoption.Candidate] = []
            for candidate in candidates {
                guard let row = rows[candidate.rowId] else { continue }
                // nil when no explorer answered, so the pair is retried rather than rejected.
                switch try? await checker.receiveCheck(for: candidate.txid, paysTo: row.address).value?.paysToAddress {
                case true?: paying.append(candidate)
                case false?: self?.rejectedTxidAdoptions.insert(candidate)
                case nil: break
                }
            }
            guard let self, self.databaseService === databaseService else { return }
            for (rowId, txid) in ReceiveTxidAdoption.adoptions(paying: paying) {
                guard let row = rows[rowId],
                      (try? databaseService.onchainRepo.attachTxid(
                          rowId: rowId,
                          txid: txid,
                          address: row.address,
                          amountMsat: row.amountMsat
                      )) == true
                else { continue }
                AuditService.log("ONCHAIN_RECEIVE_TXID_RESOLVED", data: ["txid": txid, "row": "\(rowId)"])
            }
        }
    }
}
