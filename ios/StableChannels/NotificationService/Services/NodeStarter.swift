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
        // Prefer the URL the main app last started a node against (written on every
        // successful start, including failover) so the NSE doesn't pay a degraded
        // primary's failure before falling over itself.
        let stored = UserDefaults(suiteName: Constants.appGroup)?.string(forKey: "esplora_chain_url")
        let primary = stored ?? Constants.primaryChainURL
        let fallback = (primary == Constants.primaryChainURL)
            ? Constants.fallbackChainURL
            : Constants.primaryChainURL
        return try buildNode(
            dataDir: dataDir,
            logger: logger,
            primaryURL: primary,
            fallbackURL: fallback
        )
    }
}

enum NodeStarterError: Error {
    /// A stored seed failed BIP-39 validation. LDKNode's binding aborts the
    /// process (`try!`) on an invalid mnemonic; in the NSE that means iOS
    /// launch-throttling silently kills push processing, so fail closed instead.
    case invalidStoredMnemonic
    /// keys_seed is absent. `NodeEntropy.fromSeedPath` is read-OR-GENERATE, so
    /// calling it with a missing file would mint a fresh identity into the live
    /// data dir — the silent wrong-identity class. Fail closed instead.
    case missingSeedFile
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
        let nodeMetricsReset = Self.stripGossipFromDB(path: ldkDbPath.path)
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
        let keychain: any MnemonicStorageProtocol = WalletKeychainService.shared
        let nodeEntropy: NodeEntropy
        do {
            let words = try keychain.loadMnemonic()
            // LDKNode's binding aborts the process (try!) on an invalid mnemonic —
            // validate before it can. A corrupted stored seed must fail closed.
            guard let canonicalWords = BIP39.validatedCanonicalMnemonic(words) else {
                logger.log("ERROR: SEED_INVALID_BIP39 - keychain")
                throw NodeStarterError.invalidStoredMnemonic
            }
            nodeEntropy = NodeEntropy.fromBip39Mnemonic(mnemonic: canonicalWords, passphrase: nil)
        } catch WalletKeychainError.keyNotFound {
            // Mnemonic not in Keychain: fallback check legacy plaintext file or keys_seed
            if let plaintextWords = try? String(
                contentsOfFile: dataDir.appendingPathComponent("seed_phrase").path,
                encoding: .utf8
            ),
                !plaintextWords.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let trimmed = plaintextWords.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let canonicalWords = BIP39.validatedCanonicalMnemonic(trimmed) else {
                    logger.log("ERROR: SEED_INVALID_BIP39 - plaintext")
                    throw NodeStarterError.invalidStoredMnemonic
                }
                logger.log("NOTICE: SEED_PLAINTEXT_FALLBACK - legacy_pending_migration")
                nodeEntropy = NodeEntropy.fromBip39Mnemonic(
                    mnemonic: canonicalWords,
                    passphrase: nil
                )
            } else {
                let keySeedPath = dataDir.appendingPathComponent("keys_seed")
                // fromSeedPath is read-OR-GENERATE: with the file absent it would
                // write a fresh random identity into the live data dir. Never let
                // that happen from the unattended path.
                guard FileManager.default.fileExists(atPath: keySeedPath.path) else {
                    logger.log("ERROR: SEED_FILE_MISSING - keys_seed")
                    throw NodeStarterError.missingSeedFile
                }
                nodeEntropy = try NodeEntropy.fromSeedPath(seedPath: keySeedPath.path)
            }
        } catch let error as NodeStarterError {
            // Already logged at the throw site; don't mislabel as a Keychain failure.
            throw error
        } catch {
            // Unrecoverable Keychain access failure (e.g. locked, missing group entitlements):
            // Fail immediately instead of generating a new wallet seed or using a wrong fallback keys_seed!
            logger.log("ERROR: KEYCHAIN_ACCESS_DENIED - \(error.localizedDescription)")
            throw error
        }

        // Sync config. Only fee estimation blocks node.start() — the wallet syncs run in
        // background tasks afterward — so only feeRateCacheUpdateTimeoutSecs is shortened,
        // to bound how long a degraded provider can hold up startup before failover. The
        // wallet-sync timeouts stay generous: a timed-out sync never updates
        // latest_lightning_wallet_sync_timestamp, and the NSE gets essentially one sync
        // attempt per run (600s interval), which the chain-freshness gate depends on for
        // long-offline wallets.
        let syncConfig = EsploraSyncConfig(
            backgroundSyncConfig: BackgroundSyncConfig(
                onchainWalletSyncIntervalSecs: 600,
                lightningWalletSyncIntervalSecs: 600,
                feeRateCacheUpdateIntervalSecs: 3600
            ),
            timeoutsConfig: SyncTimeoutsConfig(
                onchainWalletSyncTimeoutSecs: 60,
                lightningWalletSyncTimeoutSecs: 60,
                feeRateCacheUpdateTimeoutSecs: 6,
                txBroadcastTimeoutSecs: 30,
                perRequestTimeoutSecs: 15
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

    /// Returns true when the strip ran and deleted node_metrics (resetting LDK's persisted
    /// Lightning-sync timestamp along with the graph and scorer).
    private static func stripGossipFromDB(path: String) -> Bool {
        var db: OpaquePointer?
        guard sqlite3_open(path, &db) == SQLITE_OK else { return false }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
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
        return hasGraph
    }
}

// Keep in sync with StableChannels/Services/NodeService.swift.
extension Error {
    /// Determines whether a startup error is an Esplora feerate estimation failure or timeout
    /// eligible for provider failover. Non-Esplora errors (database, storage, lock, entropy)
    /// return false and should not be retried.
    var isRetryableEsploraStartupError: Bool {
        // Typed match only: NodeError's description is a human sentence ("Failed to
        // update fee rate estimates."), so string-matching case names never fires for
        // real errors and could false-positive on a bridged error carrying the text.
        guard let nodeError = self as? NodeError else { return false }
        switch nodeError {
        case .FeerateEstimationUpdateFailed, .FeerateEstimationUpdateTimeout:
            return true
        default:
            return false
        }
    }
}
