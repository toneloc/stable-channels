import Foundation

/// Service responsible for managing background polling of Esplora to resolve
/// pending transaction IDs. Uses staggered launchers to avoid rate limits.
@MainActor
final class TxidResolutionService {
    enum Event {
        case channelClose(opId: String, txid: String)
        case onchainReceive(resolutionId: Int64, txid: String)
    }

    // Owned by AppState.
    weak var databaseService: DatabaseService?

    private var closeTxidResolver: CloseTxidResolver?
    private var onchainTxidResolver: OnchainTxidResolver?
    private var closeLauncher = StaggeredTaskLauncher()
    private var onchainLauncher = StaggeredTaskLauncher()

    var didResolveTxid: ((Event) -> Void)?

    func configureResolvers(urls: [String]) {
        precondition(
            closeTxidResolver == nil && onchainTxidResolver == nil,
            "Resolvers should only be configured once"
        )
        let resolverConfig = URLSessionConfiguration.default
        resolverConfig.timeoutIntervalForRequest = 5
        resolverConfig.timeoutIntervalForResource = 10
        let resolverSession = URLSession(configuration: resolverConfig)

        closeTxidResolver = CloseTxidResolver(
            chainURLs: urls,
            onResolved: { [weak self] opId, closingTxid in
                self?.didResolveTxid?(.channelClose(opId: opId, txid: closingTxid))
            },
            urlSession: resolverSession
        )

        onchainTxidResolver = OnchainTxidResolver(
            chainURLs: urls,
            onResolved: { [weak self] workId, txid in
                guard let idStr = workId.split(separator: "-", maxSplits: 1).last,
                      let resolutionId = Int64(idStr) else { return }
                self?.didResolveTxid?(.onchainReceive(resolutionId: resolutionId, txid: txid))
            },
            urlSession: resolverSession
        )
    }

    func clearResolvers() {
        cancelAllLaunchers()
        closeTxidResolver = nil
        onchainTxidResolver = nil
    }

    func cancelAllLaunchers() {
        closeLauncher.cancelAll()
        onchainLauncher.cancelAll()
    }

    func startCloseTxidResolver(opId: String, delaySeconds: UInt64 = 0) {
        guard closeTxidResolver != nil else {
            AuditService.log("RESOLVER_NOT_CONFIGURED", data: ["type": "close", "opId": opId])
            return
        }
        closeLauncher.launch(opId: opId, delaySeconds: delaySeconds) { [weak self] in
            guard let self, let db = self.databaseService else { return }
            await self.closeTxidResolver?.resolve(opId: opId, databaseService: db)
        }
    }

    func startOnchainTxidResolver(resolutionId: Int64, address: String, delaySeconds: UInt64 = 0) {
        guard onchainTxidResolver != nil else {
            AuditService.log("RESOLVER_NOT_CONFIGURED", data: ["type": "onchain", "resolutionId": "\(resolutionId)"])
            return
        }
        let opId = "onchain-receive-\(resolutionId)"
        onchainLauncher.launch(opId: opId, delaySeconds: delaySeconds) { [weak self] in
            guard let self, let db = self.databaseService else { return }
            await self.onchainTxidResolver?.resolve(
                resolutionId: resolutionId,
                address: address,
                databaseService: db
            )
        }
    }

    func replayPendingChannelCloses() {
        guard let db = databaseService else { return }
        let pending = db.fetchPendingOperations()
        for (i, op) in pending.enumerated() where op.opType == "channel_close" {
            let delay = UInt64(i)
            startCloseTxidResolver(opId: op.opId, delaySeconds: delay)
        }
    }

    func replayPendingOnchainReceives() {
        guard let db = databaseService else { return }
        let pending = db.fetchPendingOnchainReceives()
        for (i, res) in pending.enumerated() {
            let delay = UInt64(i)
            startOnchainTxidResolver(resolutionId: res.id, address: res.address, delaySeconds: delay)
        }
    }
}
