import Foundation
import LDKNode

@Observable
class NodeService {
    private(set) var node: Node?
    private(set) var isRunning = false
    private(set) var nodeId: String = ""
    private(set) var channels: [ChannelDetails] = []
    private var eventTask: Task<Void, Never>?

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

        if mnemonic.isEmpty {
            // New wallet or existing seed on disk — ldk-node handles it
        } else {
            builder.setEntropyBip39Mnemonic(mnemonic: mnemonic, passphrase: nil)
        }

        let ldkNode = try builder.build()
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
            while !Task.isCancelled {
                let event = await node.nextEventAsync()
                if Task.isCancelled { break }

                await MainActor.run {
                    NotificationCenter.default.post(
                        name: .ldkEventReceived,
                        object: event
                    )
                }
                try? node.eventHandled()
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

    // MARK: - Splice Operations

    func spliceIn(userChannelId: UserChannelId, counterpartyNodeId: PublicKey, amountSats: UInt64) throws {
        guard let node else { throw NodeServiceError.notRunning }
        try node.spliceIn(userChannelId: userChannelId, counterpartyNodeId: counterpartyNodeId, spliceAmountSats: amountSats)
    }

    func spliceOut(userChannelId: UserChannelId, counterpartyNodeId: PublicKey, address: String, amountSats: UInt64) throws {
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

    func sendBolt12(offer: Offer, amountMsat: UInt64) throws -> PaymentId {
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
            maxLspFeeLimitMsat: nil
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
        return try node.onchainPayment().sendAllToAddress(address: address, retainReserve: false, feeRate: nil)
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
