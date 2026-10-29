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
    private let nowSecs: () -> Int64
    private var lastMissedReceiveCheckSecs: Int64 = 0
    private var rejectedTxidAdoptions = Set<ReceiveTxidAdoption.Candidate>()
    private var isResolvingTxidlessReceives = false

    init(
        transactionLinkService: TransactionLinkService,
        makeClient: @escaping () -> ResilientEsploraClient,
        nowSecs: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.transactionLinkService = transactionLinkService
        self.makeClient = makeClient
        self.nowSecs = nowSecs
    }

    /// Throttled explorer pass over the receive address. Records by txid, so it is safe during a
    /// splice: the repository skips the splice's own txid and refuses while a splice has no txid.
    func recoverMissedReceive(address: String?, price: Double) {
        guard let address, !address.isEmpty, let databaseService else { return }
        let now = nowSecs()
        guard MissedReceiveRecovery.isCheckDue(lastCheckSecs: lastMissedReceiveCheckSecs, nowSecs: now)
        else { return }
        lastMissedReceiveCheckSecs = now
        let client = makeClient()
        Task { @MainActor [weak self] in
            let receives = await client.fetch(
                endpointBuilder: { ["\(ResilientEsploraClient.trimSlash($0))/address/\(address)/txs"] },
                resultParser: { MissedReceiveRecovery.recentReceives(json: $0, address: address, nowSecs: now) }
            )
            guard let self, let receives, self.databaseService === databaseService else { return }
            for receive in receives {
                do {
                    let recorded = try databaseService.onchainRepo.recordMissedReceive(
                        txid: receive.txid,
                        amountSats: receive.sats,
                        amountUSD: price > 0 ? BitcoinUnits.usd(sats: receive.sats, price: price) : nil,
                        btcPrice: price > 0 ? price : nil,
                        address: address,
                        sinceSecs: now - MissedReceiveRecovery.windowSeconds
                    )
                    guard recorded else { continue }
                    // Lets the balance-delta path see this txid already has a row, as the
                    // websocket path does, instead of adding a second placeholder row.
                    transactionLinkService.setReceiveTxid(receive.txid, address: address)
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

    /// Adopts a wallet-seen txid for a pending placeholder, only once that txid is proven to pay
    /// the placeholder's address. One pass runs at a time; disproven pairs are never retried.
    func resolveTxidlessReceives(walletReceives: () -> [ReceiveTxidAdoption.WalletReceive]) {
        guard !isResolvingTxidlessReceives, let databaseService else { return }
        let placeholders = databaseService.onchainRepo.txidlessPendingReceivesWithAddress()
        guard !placeholders.isEmpty,
              let recorded = try? databaseService.onchainRepo.recordedReceiveTxids() else { return }
        let candidates = ReceiveTxidAdoption.candidates(
            placeholders: placeholders,
            walletReceives: walletReceives(),
            recordedTxids: recorded
        ).filter { !rejectedTxidAdoptions.contains($0) }
        guard !candidates.isEmpty else { return }
        let addresses = Dictionary(uniqueKeysWithValues: placeholders.map { ($0.id, $0.address) })
        let client = makeClient()
        isResolvingTxidlessReceives = true
        Task { @MainActor [weak self] in
            defer { self?.isResolvingTxidlessReceives = false }
            var paying: [ReceiveTxidAdoption.Candidate] = []
            for candidate in candidates {
                guard let address = addresses[candidate.rowId] else { continue }
                // Amount alone is ambiguous (the wallet history can hold old transactions of
                // the same size), so only a txid that pays the row's address qualifies.
                switch await Self.paysToAddress(txid: candidate.txid, address: address, client: client) {
                case true?: paying.append(candidate)
                case false?: self?.rejectedTxidAdoptions.insert(candidate)
                case nil: break
                }
            }
            guard let self, self.databaseService === databaseService else { return }
            for (rowId, txid) in ReceiveTxidAdoption.adoptions(paying: paying) {
                guard let address = addresses[rowId],
                      databaseService.onchainRepo.adoptTxid(rowId: rowId, txid: txid, address: address)
                else { continue }
                AuditService.log("ONCHAIN_RECEIVE_TXID_RESOLVED", data: ["txid": txid, "row": "\(rowId)"])
            }
        }
    }

    /// nil when no explorer answered, so the pair is retried rather than rejected.
    private nonisolated static func paysToAddress(
        txid: String,
        address: String,
        client: ResilientEsploraClient
    ) async -> Bool? {
        await client.fetch(
            endpointBuilder: { ["\(ResilientEsploraClient.trimSlash($0))/tx/\(txid)"] },
            resultParser: { data in
                guard let tx = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
                return MissedReceiveRecovery.satsPaidTo(tx, address: address) > 0
            }
        )
    }
}
