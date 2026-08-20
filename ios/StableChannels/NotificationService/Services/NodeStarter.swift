import Foundation
import LDKNode
import SQLite3

/// Protocol for starting an LDK node
protocol NodeStarter {
    func buildNode(dataDir: URL, logger: Logger, primaryURL: String, fallbackURL: String) throws -> LDKNode.Node
    func connectToLSP(node: LDKNode.Node) throws
}

extension NodeStarter {
    func buildNode(dataDir: URL, logger: Logger) throws -> LDKNode.Node {
        try buildNode(
            dataDir: dataDir,
            logger: logger,
            primaryURL: Constants.primaryChainURL,
            fallbackURL: Constants.fallbackChainURL
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

        Self.stripGossipFromDB(path: ldkDbPath.path)

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

        // Fast sync config for background execution under strict 30s deadline
        let syncConfig = EsploraSyncConfig(
            backgroundSyncConfig: BackgroundSyncConfig(
                onchainWalletSyncIntervalSecs: 600,
                lightningWalletSyncIntervalSecs: 600,
                feeRateCacheUpdateIntervalSecs: 3600
            ),
            timeoutsConfig: SyncTimeoutsConfig(
                onchainWalletSyncTimeoutSecs: 6,
                lightningWalletSyncTimeoutSecs: 6,
                feeRateCacheUpdateTimeoutSecs: 6,
                txBroadcastTimeoutSecs: 10,
                perRequestTimeoutSecs: 4
            )
        )

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

    private static func stripGossipFromDB(path: String) {
        var db: OpaquePointer?
        guard sqlite3_open(path, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return }
        let hasGraph: Bool
        if sqlite3_step(stmt) == SQLITE_ROW {
            let size = sqlite3_column_int64(stmt, 0)
            hasGraph = size > 100_000
        } else {
            hasGraph = false
        }
        sqlite3_finalize(stmt)

        if hasGraph {
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'scorer'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'node_metrics'", nil, nil, nil)
            sqlite3_exec(db, "VACUUM", nil, nil, nil)
        }
    }
}

// Keep in sync with StableChannels/Services/NodeService.swift.
extension Error {
    /// Determines whether a startup error is an Esplora feerate estimation failure or timeout
    /// eligible for provider failover. Non-Esplora errors (database, storage, lock, entropy)
    /// return false and should not be retried.
    var isRetryableEsploraStartupError: Bool {
        if let nodeError = self as? NodeError {
            switch nodeError {
            case .FeerateEstimationUpdateFailed, .FeerateEstimationUpdateTimeout:
                return true
            default:
                return false
            }
        }
        let desc = localizedDescription
        return desc.contains("FeerateEstimationUpdateFailed") || desc.contains("FeerateEstimationUpdateTimeout")
    }
}
