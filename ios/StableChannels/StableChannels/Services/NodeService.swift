import Foundation
import LDKNode

/// Passed in notification userInfo so the handler can veto the eventHandled() call.
/// NotificationCenter.post() is synchronous on MainActor, so all observers run
/// before NodeService checks shouldAck — no race condition.
final class EventAckToken {
    var shouldAck = true
}

@Observable
class NodeService {
    private(set) var node: Node?
    private(set) var isRunning = false
    private(set) var nodeId: String = ""
    private(set) var channels: [ChannelDetails] = []
    private(set) var savedMnemonic: String?
    private var eventTask: Task<Void, Never>?

    weak var databaseService: DatabaseService?

    init() {
        // Pre-load saved mnemonic from disk so it's available immediately,
        // even before start() completes (avoids race with early UI display)
        let path = Constants.userDataDir.appendingPathComponent("seed_phrase")
        if let words = try? String(contentsOfFile: path.path, encoding: .utf8) {
            let trimmed = words.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                savedMnemonic = trimmed
            }
        }
    }

    // MARK: - Lifecycle

    func start(network: Network, esploraURL: String, mnemonic: String) async throws {
        let dataDir = Constants.userDataDir.path

        // Ensure data directory exists
        try? FileManager.default.createDirectory(atPath: dataDir, withIntermediateDirectories: true)

        var config = defaultConfig()
        config.storageDirPath = dataDir
        config.network = network
        config.trustedPeers0conf = [Constants.defaultLSPPubkey]

        // Anchor channels: trust LSP so no reserve held for their channel
        config.anchorChannelsConfig = AnchorChannelsConfig(
            trustedPeersNoReserve: [Constants.defaultLSPPubkey],
            perChannelReserveSats: 25_000
        )

        let builder = Builder.fromConfig(config: config)

        let logPath = Constants.userDataDir.appendingPathComponent("ldk-node.log").path
        builder.setFilesystemLogger(logFilePath: logPath, maxLogLevel: .debug)

        let syncConfig = EsploraSyncConfig(
            backgroundSyncConfig: BackgroundSyncConfig(
                onchainWalletSyncIntervalSecs: Constants.onchainWalletSyncIntervalSecs,
                lightningWalletSyncIntervalSecs: Constants.lightningWalletSyncIntervalSecs,
                feeRateCacheUpdateIntervalSecs: Constants.feeRateCacheUpdateIntervalSecs
            ),
            timeoutsConfig: SyncTimeoutsConfig(
                onchainWalletSyncTimeoutSecs: 60,
                lightningWalletSyncTimeoutSecs: 60,
                feeRateCacheUpdateTimeoutSecs: 60,
                txBroadcastTimeoutSecs: 30,
                perRequestTimeoutSecs: 15
            )
        )
        builder.setChainSourceEsplora(serverUrl: esploraURL, config: syncConfig)

        switch network {
        case .bitcoin:
            builder.setGossipSourceRgs(rgsServerUrl: Constants.RGSServer.bitcoin)
        case .signet:
            builder.setGossipSourceRgs(rgsServerUrl: Constants.RGSServer.signet)
        case .testnet:
            builder.setGossipSourceRgs(rgsServerUrl: Constants.RGSServer.testnet)
        default:
            break
        }

        // LSPS2 liquidity source — enables JIT channel opening on first receive
        builder.setLiquiditySourceLsps2(
            nodeId: Constants.defaultLSPPubkey,
            address: Constants.defaultLSPAddress,
            token: nil
        )

        let seedPhrasePath = Constants.userDataDir.appendingPathComponent("seed_phrase")
        let keySeedPath = Constants.userDataDir.appendingPathComponent("keys_seed")

        // Determine which mnemonic to use
        let words: String
        if !mnemonic.isEmpty {
            // Restore — wipe ALL wallet data so new seed takes effect
            Self.wipeWalletData()
            words = mnemonic.trimmingCharacters(in: .whitespacesAndNewlines)
        } else if let saved = try? String(contentsOfFile: seedPhrasePath.path, encoding: .utf8),
                  !saved.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            // Existing wallet — re-read saved mnemonic
            words = saved.trimmingCharacters(in: .whitespacesAndNewlines)
        } else if !FileManager.default.fileExists(atPath: keySeedPath.path) {
            // Truly new wallet — no seed_phrase, no keys_seed
            Self.wipeWalletData()
            words = generateEntropyMnemonic(wordCount: nil)
        } else {
            // Pre-upgrade wallet with only keys_seed, no mnemonic available
            words = ""
        }

        // Save mnemonic to file and derive node entropy (now passed to build()).
        let nodeEntropy: NodeEntropy
        if !words.isEmpty {
            try words.write(toFile: seedPhrasePath.path, atomically: true, encoding: .utf8)
            self.savedMnemonic = words
            nodeEntropy = NodeEntropy.fromBip39Mnemonic(mnemonic: words, passphrase: nil)
        } else {
            // Pre-upgrade wallet with only keys_seed: derive entropy from that seed file.
            nodeEntropy = try NodeEntropy.fromSeedPath(seedPath: keySeedPath.path)
        }

        let ldkNode = try builder.build(nodeEntropy: nodeEntropy)
        try ldkNode.start()

        self.node = ldkNode
        self.isRunning = true
        self.nodeId = ldkNode.nodeId()

        // Connect to gateway node (well-connected Lightning node for routing)
        try? ldkNode.connect(
            nodeId: Constants.defaultGatewayPubkey,
            address: Constants.defaultGatewayAddress,
            persist: true
        )

        // Connect to LSP
        try? ldkNode.connect(
            nodeId: Constants.defaultLSPPubkey,
            address: Constants.defaultLSPAddress,
            persist: true
        )

        refreshChannels()
        startEventLoop()
    }

    func stop() {
        eventTask?.cancel()
        eventTask = nil
        try? node?.stop()
        isRunning = false
    }

    // MARK: - Event Loop

    private func startEventLoop() {
        eventTask?.cancel()
        eventTask = Task { [weak self] in
            guard let self, let node = self.node else { return }
            var retryDelayNanoseconds: UInt64 = 1_000_000_000
            while !Task.isCancelled {
                let event = await node.nextEventAsync()
                if Task.isCancelled { break }

                let shouldAck = await MainActor.run {
                    let token = EventAckToken()
                    NotificationCenter.default.post(
                        name: .ldkEventReceived,
                        object: event,
                        userInfo: ["ackToken": token]
                    )
                    return token.shouldAck
                }
                if shouldAck {
                    try? node.eventHandled()
                    retryDelayNanoseconds = 1_000_000_000
                } else {
                    try? await Task.sleep(nanoseconds: retryDelayNanoseconds)
                    retryDelayNanoseconds = min(retryDelayNanoseconds * 2, 30_000_000_000)
                }
            }
        }
    }

    // MARK: - Channel Operations

    func refreshChannels() {
        guard let node else { return }
        channels = node.listChannels()
    }

    func connectAndOpenChannel(
        pubkey: String,
        address: String,
        amountSats: UInt64
    ) async throws {
        guard let node else { throw NodeServiceError.notRunning }
        try node.connect(nodeId: pubkey, address: address, persist: true)
        _ = try node.openChannel(
            nodeId: pubkey,
            address: address,
            channelAmountSats: amountSats,
            pushToCounterpartyMsat: nil,
            channelConfig: nil
        )
        refreshChannels()
    }

    func closeChannel(userChannelId: UserChannelId, counterpartyNodeId: PublicKey) throws {
        guard let node else { throw NodeServiceError.notRunning }
        try node.closeChannel(userChannelId: userChannelId, counterpartyNodeId: counterpartyNodeId)
    }

    func requestChannelClose(
        userChannelId: UserChannelId,
        counterpartyNodeId: PublicKey,
        fundingOutpointTxid: String,
        fundingOutpointVout: UInt32,
        balanceSats: UInt64,
        balanceUsd: Double?,
        btcPrice: Double?,
        counterparty: String?
    ) async throws {
        guard let node else { throw NodeServiceError.notRunning }

        // Persist intent + snapshot first: resolver reads from this row at resolve time
        let opId = "close-\(userChannelId)"
        databaseService?.insertPendingOperation(
            opId: opId,
            opType: "channel_close",
            fundingOutpointTxid: fundingOutpointTxid,
            fundingOutpointVout: fundingOutpointVout,
            balanceSats: balanceSats,
            balanceUsd: balanceUsd,
            btcPrice: btcPrice,
            counterparty: counterparty
        )

        AuditService.log("CHANNEL_CLOSE_REQUESTED", data: [
            "user_channel_id": "\(userChannelId)",
            "funding_outpoint": "\(fundingOutpointTxid):\(fundingOutpointVout)"
        ])

        do {
            try node.closeChannel(
                userChannelId: userChannelId,
                counterpartyNodeId: counterpartyNodeId
            )
        } catch {
            databaseService?.updatePendingOperation(
                opId: opId,
                closingTxid: "",
                status: "failed"
            )
            AuditService.log("CHANNEL_CLOSE_REQUEST_FAILED", data: [
                "user_channel_id": "\(userChannelId)",
                "error": "\(error)"
            ])
            throw error
        }
    }

    // MARK: - Splice Operations

    func spliceIn(userChannelId: UserChannelId, counterpartyNodeId: PublicKey, amountSats: UInt64) throws {
        guard let node else { throw NodeServiceError.notRunning }
        try node.spliceIn(
            userChannelId: userChannelId,
            counterpartyNodeId: counterpartyNodeId,
            spliceAmountSats: amountSats
        )
    }

    func spliceOut(
        userChannelId: UserChannelId,
        counterpartyNodeId: PublicKey,
        address: String,
        amountSats: UInt64
    ) throws {
        guard let node else { throw NodeServiceError.notRunning }
        try node.spliceOut(
            userChannelId: userChannelId,
            counterpartyNodeId: counterpartyNodeId,
            address: address,
            spliceAmountSats: amountSats
        )
    }

    // MARK: - Payments

    func sendPayment(invoice: Bolt11Invoice) throws -> PaymentId {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt11Payment().send(invoice: invoice, routeParameters: nil)
    }

    func sendPaymentUsingAmount(invoice: Bolt11Invoice, amountMsat: UInt64) throws -> PaymentId {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt11Payment().sendUsingAmount(invoice: invoice, amountMsat: amountMsat, routeParameters: nil)
    }

    func sendBolt12(offer: Offer, amountMsat _: UInt64) throws -> PaymentId {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt12Payment().send(
            offer: offer,
            quantity: nil,
            payerNote: nil,
            routeParameters: nil
        )
    }

    func sendBolt12UsingAmount(offer: Offer, amountMsat: UInt64) throws -> PaymentId {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt12Payment().sendUsingAmount(
            offer: offer,
            amountMsat: amountMsat,
            quantity: nil,
            payerNote: nil,
            routeParameters: nil
        )
    }

    func sendKeysend(amountMsat: UInt64, to nodeId: PublicKey) throws -> PaymentId {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.spontaneousPayment().send(amountMsat: amountMsat, nodeId: nodeId, routeParameters: nil)
    }

    func sendKeysendWithTLV(
        amountMsat: UInt64,
        to nodeId: PublicKey,
        tlvs: [CustomTlvRecord]
    ) throws -> PaymentId {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.spontaneousPayment().sendWithCustomTlvs(
            amountMsat: amountMsat,
            nodeId: nodeId,
            routeParameters: nil,
            customTlvs: tlvs
        )
    }

    func receivePayment(amountMsat: UInt64, description: String) throws -> Bolt11Invoice {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt11Payment().receive(
            amountMsat: amountMsat,
            description: .direct(description: description),
            expirySecs: Constants.invoiceExpirySecs
        )
    }

    func receiveVariablePayment(description: String) throws -> Bolt11Invoice {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt11Payment().receiveVariableAmount(
            description: .direct(description: description),
            expirySecs: Constants.invoiceExpirySecs
        )
    }

    /// Receive via JIT channel (LSPS2) — for users without a channel yet
    func receiveViaJitChannel(amountMsat: UInt64, description: String) throws -> Bolt11Invoice {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.bolt11Payment().receiveViaJitChannel(
            amountMsat: amountMsat,
            description: .direct(description: description),
            expirySecs: Constants.invoiceExpirySecs,
            maxTotalLspFeeLimitMsat: nil
        )
    }

    // MARK: - On-Chain

    func newOnchainAddress() throws -> String {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.onchainPayment().newAddress()
    }

    func sendOnchain(address: String, amountSats: UInt64) throws -> Txid {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.onchainPayment().sendToAddress(address: address, amountSats: amountSats, feeRate: nil)
    }

    func sendAllOnchain(address: String) throws -> Txid {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.onchainPayment().sendAllToAddress(address: address, retainReserves: false, feeRate: nil)
    }

    // MARK: - Balances

    func balances() -> BalanceDetails? {
        node?.listBalances()
    }

    func spendableOnchainSats() -> UInt64 {
        node?.listBalances().spendableOnchainBalanceSats ?? 0
    }

    // MARK: - Wallet

    func signMessage(_ message: [UInt8]) throws -> String {
        guard let node else { throw NodeServiceError.notRunning }
        return try node.signMessage(msg: message)
    }

    func verifySignature(message: [UInt8], signature: String, pubkey: PublicKey) -> Bool {
        node?.verifySignature(msg: message, sig: signature, pkey: pubkey) ?? false
    }

    /// Wipe all wallet data files (keys_seed, SQLite + journals, seed_phrase)
    /// so a fresh wallet can be created without descriptor conflicts.
    static func wipeWalletData() {
        let dir = Constants.userDataDir
        let filesToDelete = [
            "keys_seed",
            "seed_phrase",
            "ldk_node_data.sqlite",
            "ldk_node_data.sqlite-wal",
            "ldk_node_data.sqlite-shm"
        ]
        for file in filesToDelete {
            try? FileManager.default.removeItem(at: dir.appendingPathComponent(file))
        }
    }
}

enum NodeServiceError: LocalizedError {
    case notRunning

    var errorDescription: String? {
        switch self {
        case .notRunning: return "Node is not running"
        }
    }
}

extension Notification.Name {
    static let ldkEventReceived = Notification.Name("ldkEventReceived")
}
