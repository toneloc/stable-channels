import Foundation
import LDKNode
import SQLite3

/// Protocol for starting an LDK node
protocol NodeStarter {
    func buildNode(dataDir: URL, logger: Logger) throws -> LDKNode.Node
    func connectToLSP(node: LDKNode.Node) throws
}

/// Concrete implementation of NodeStarter
final class DefaultNodeStarter: NodeStarter {
    private static let lspPubkey = Constants.lspPubkey
    private static let lspAddress = Constants.lspAddress

    func buildNode(dataDir: URL, logger: Logger) throws -> LDKNode.Node {
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

        // Sync config
        let syncConfig = EsploraSyncConfig(
            backgroundSyncConfig: BackgroundSyncConfig(
                onchainWalletSyncIntervalSecs: 600,
                lightningWalletSyncIntervalSecs: 600,
                feeRateCacheUpdateIntervalSecs: 3600
            ),
            timeoutsConfig: SyncTimeoutsConfig(
                onchainWalletSyncTimeoutSecs: 60,
                lightningWalletSyncTimeoutSecs: 60,
                feeRateCacheUpdateTimeoutSecs: 60,
                txBroadcastTimeoutSecs: 30,
                perRequestTimeoutSecs: 15
            )
        )

        let builder = LDKNode.Builder.fromConfig(config: config)
        builder.setChainSourceEsplora(
            serverUrl: "https://blockstream.info/api",
            config: syncConfig
        )

        let node = try builder.build(nodeEntropy: nodeEntropy)

        let memAfterBuild = Diagnostics.residentMemoryBytes()
        logger.log("Mem after build: \(memAfterBuild / 1024 / 1024) MB")

        try node.start()

        let memAfterStart = Diagnostics.residentMemoryBytes()
        logger.log("Mem after start: \(memAfterStart / 1024 / 1024) MB")

        return node
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
