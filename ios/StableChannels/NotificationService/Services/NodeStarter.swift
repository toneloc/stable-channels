import Foundation
import LDKNode

/// Protocol for starting an LDK node
protocol NodeStarter {
    func buildNode(dataDir: URL, logger: Logger, primaryURL: String, fallbackURL: String) throws -> LDKNode.Node
    func connectToLSP(node: LDKNode.Node) throws
}

extension NodeStarter {
    func buildNode(dataDir: URL, logger: Logger) throws -> LDKNode.Node {
        // Prefer the URL the main app last started a node against (written on every
        // successful start, including failover) so the NSE doesn't pay a degraded
        // primary's failure before falling over itself.
        let stored = UserDefaultsChainURLStore(defaults: UserDefaults(suiteName: Constants.appGroup)).load()
        let providers = NodeStartupPolicy.extensionProviders(
            preferred: stored ?? Constants.primaryChainURL,
            providers: Constants.esploraChainURLs
        )
        return try buildNode(
            dataDir: dataDir,
            logger: logger,
            primaryURL: providers[0],
            fallbackURL: providers[1]
        )
    }
}

/// Concrete implementation of NodeStarter
final class DefaultNodeStarter: NodeStarter {
    private static let lspPubkey = Constants.lspPubkey
    private static let lspAddress = Constants.lspAddress

    func buildNode(
        dataDir: URL,
        logger: Logger,
        primaryURL: String = Constants.primaryChainURL,
        fallbackURL: String = Constants.fallbackChainURL
    ) throws -> LDKNode.Node {
        let memBefore = Diagnostics.residentMemoryBytes()
        logger.log("Mem before build: \(memBefore / 1024 / 1024) MB")

        // Strip gossip first
        let ldkDbPath = dataDir.appendingPathComponent("ldk_node_data.sqlite")

        let attrs = try? FileManager.default.attributesOfItem(atPath: ldkDbPath.path)
        let dbSize = (attrs?[.size] as? UInt64) ?? 0
        logger.log("ldk_node_data size: \(dbSize / 1024 / 1024) MB")

        // A strip deletes node_metrics along with the graph, resetting LDK's persisted
        // latest_lightning_wallet_sync_timestamp — so on strip runs the freshness gate must
        // wait for a new sync instead of inheriting the main app's recent one. Logged so the
        // pilot metrics can split immediate-send runs from forced-resync runs.
        let nodeMetricsReset = LdkDbMaintenance(dataDir: dataDir, log: logger.log).stripGossip()
        logger
            .log("stability_gate {\"event\":\"node_metrics_reset\",\"platform\":\"ios\",\"reset\":\(nodeMetricsReset)}")

        // Node config
        var config = LDKNode.defaultConfig()
        config.storageDirPath = dataDir.path
        config.network = .bitcoin
        config.trustedPeers0conf = [Self.lspPubkey]
        config.anchorChannelsConfig = LDKNode.AnchorChannelsConfig(
            trustedPeersNoReserve: [Self.lspPubkey],
            perChannelReserveSats: 25_000
        )

        // Derive node entropy
        let nodeEntropy: NodeEntropy
        let seedPhrasePath = dataDir.appendingPathComponent("seed_phrase")
        if FileManager.default.fileExists(atPath: seedPhrasePath.path),
           let words = (try? String(contentsOfFile: seedPhrasePath.path, encoding: .utf8))?
           .trimmingCharacters(in: .whitespacesAndNewlines),
           !words.isEmpty {
            nodeEntropy = NodeEntropy.fromBip39Mnemonic(mnemonic: words, passphrase: nil)
        } else {
            let keySeedPath = dataDir.appendingPathComponent("keys_seed")
            nodeEntropy = try NodeEntropy.fromSeedPath(seedPath: keySeedPath.path)
        }

        // Sync config. Only fee estimation blocks node.start() — the wallet syncs run in
        // background tasks afterward — so only feeRateCacheUpdateTimeoutSecs is shortened,
        // to bound how long a degraded provider can hold up startup before failover. The
        // wallet-sync timeouts stay generous: a timed-out sync never updates
        // latest_lightning_wallet_sync_timestamp, and the NSE gets essentially one sync
        // attempt per run (600s interval), which the chain-freshness gate depends on for
        // long-offline wallets.
        let syncConfig = Constants.esploraSyncConfig

        do {
            let builder = LDKNode.Builder.fromConfig(config: config)
            builder.setChainSourceEsplora(
                serverUrl: primaryURL,
                config: syncConfig
            )
            let node = try builder.build(nodeEntropy: nodeEntropy)
            let memAfterBuild = Diagnostics.residentMemoryBytes()
            logger.log("Mem after build: \(memAfterBuild / 1024 / 1024) MB")

            try node.start()

            let memAfterStart = Diagnostics.residentMemoryBytes()
            logger.log("Mem after start: \(memAfterStart / 1024 / 1024) MB")
            return node
        } catch {
            guard error.isRetryableEsploraStartupError else {
                logger.log("Non-retryable startup error in NSE: \(error). Propagating immediately.")
                throw error
            }

            let initialError = error
            logger
                .log(
                    "Primary Esplora failed during start: \(initialError.localizedDescription). Retrying with fallback: \(fallbackURL)"
                )
            do {
                let fallbackBuilder = LDKNode.Builder.fromConfig(config: config)
                fallbackBuilder.setChainSourceEsplora(
                    serverUrl: fallbackURL,
                    config: syncConfig
                )
                let fallbackNode = try fallbackBuilder.build(nodeEntropy: nodeEntropy)
                try fallbackNode.start()

                let memAfterStart = Diagnostics.residentMemoryBytes()
                logger.log("Mem after fallback start: \(memAfterStart / 1024 / 1024) MB")
                return fallbackNode
            } catch let fallbackError {
                logger
                    .log(
                        "Fallback Esplora also failed in NSE: \(fallbackError.localizedDescription). Preserving primary error."
                    )
                throw initialError
            }
        }
    }

    func connectToLSP(node: LDKNode.Node) throws {
        try node.connect(
            nodeId: Self.lspPubkey,
            address: Self.lspAddress,
            persist: true
        )
    }
}
