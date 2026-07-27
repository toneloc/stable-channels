import Foundation
import os.log

// MARK: - Model

struct MempoolWSBlock: Decodable {
    let height: UInt32
}

struct MempoolWSVout: Decodable {
    let scriptpubkeyAddress: String?
    let value: Int64?

    enum CodingKeys: String, CodingKey {
        case scriptpubkeyAddress = "scriptpubkey_address"
        case value
    }
}

struct MempoolWSVin: Decodable {
    let txid: String?
}

struct MempoolWSTransaction: Decodable {
    let txid: String
    let vout: [MempoolWSVout]?
    let vin: [MempoolWSVin]?
}

struct MempoolWSAddressTransactions: Decodable {
    let mempool: [MempoolWSTransaction]?
    let confirmed: [MempoolWSTransaction]?
    let removed: [MempoolWSTransaction]?
}

struct MempoolWSOutspend: Decodable {
    let txid: String
    let vin: Int
}

struct MempoolWSTxTrackingInfo: Decodable {
    let utxoSpent: [String: MempoolWSOutspend]?
    let confirmed: Bool?
}

struct MempoolWSMessage: Decodable {
    let block: MempoolWSBlock?
    let blocks: [MempoolWSBlock]?
    let addressTransactions: [MempoolWSTransaction]?
    let blockTransactions: [MempoolWSTransaction]?
    let address: String?
    let txid: String?

    // Bulk tracking payloads
    let multiAddressTransactions: [String: MempoolWSAddressTransactions]?
    let trackedTxs: [String: MempoolWSTxTrackingInfo]?

    enum CodingKeys: String, CodingKey {
        case block
        case blocks
        case addressTransactions = "address-transactions"
        case blockTransactions = "block-transactions"
        case address
        case txid

        case multiAddressTransactions = "multi-address-transactions"
        case trackedTxs = "tracked-txs"
    }
}

/// Manages a native Swift `URLSessionWebSocketTask` connection to Mempool.space
/// for real-time sub-second incoming payment alerts, txid resolution, and block tip updates.
@MainActor
final class MempoolWebSocketService: NSObject, URLSessionWebSocketDelegate, MempoolWebSocketProtocol {
    private(set) var isConnected: Bool = false
    private let wsEndpointURL: URL
    private let logger = Logger(subsystem: "com.stablechannels", category: "websocket")
    private let decoder = JSONDecoder()
    private let matcher = TransactionMatcher()

    private var urlSession: URLSession?
    private var webSocketTask: URLSessionWebSocketTask?
    private var trackedAddresses: Set<String> = []
    private var trackedTxids: Set<String> = []
    private var pendingOutboundMessages: [String] = []
    private var processedTxids: [String: Date] = [:]
    private let processedTxidTTL: TimeInterval = 900 // 15 minutes
    private var lastPurgeTime: Date = .distantPast
    private var isManualDisconnect: Bool = false
    private var reconnectTask: Task<Void, Never>?

    /// Fired when a transaction is detected hitting a tracked address or txid outspend.
    var onTransactionDetected: ((WebSocketEvent) -> Void)?

    /// Fired when a new block header is mined.
    var onBlockHeader: ((_ height: UInt32) -> Void)?

    init(endpointURL: URL = URL(string: "wss://mempool.space/api/v1/ws")!) {
        self.wsEndpointURL = endpointURL
        super.init()
    }

    // MARK: - Connection Lifecycle

    /// Establishes the WebSocket connection and starts the message listener loop.
    func connect() {
        guard !isConnected else { return }

        reconnectTask?.cancel()
        reconnectTask = nil
        webSocketTask?.cancel(with: .goingAway, reason: nil)
        webSocketTask = nil
        isManualDisconnect = false

        urlSession?.invalidateAndCancel()
        let config = URLSessionConfiguration.default
        let session = URLSession(configuration: config, delegate: self, delegateQueue: nil)
        self.urlSession = session

        webSocketTask = session.webSocketTask(with: wsEndpointURL)
        webSocketTask?.resume()

        logger.info("[WebSocket] Initiated connection to \(self.wsEndpointURL.absoluteString)")
    }

    nonisolated func urlSession(
        _: URLSession,
        webSocketTask _: URLSessionWebSocketTask,
        didOpenWithProtocol _: String?
    ) {
        Task { @MainActor in
            guard !self.isManualDisconnect else { return }
            self.isConnected = true
            self.logger.info("[WebSocket] Connected to Mempool WebSocket successfully")
            AuditService.log("WEBSOCKET_CONNECTED", data: ["url": self.wsEndpointURL.absoluteString])

            self.syncTracking()
            self.trackBlocks()
            self.flushPendingMessages()
            self.receiveMessages()
        }
    }

    /// Disconnects the WebSocket gracefully and invalidates the session.
    func disconnect() {
        isManualDisconnect = true
        reconnectTask?.cancel()
        reconnectTask = nil

        webSocketTask?.cancel(with: .goingAway, reason: nil)
        webSocketTask = nil
        urlSession?.invalidateAndCancel()
        urlSession = nil
        isConnected = false
        logger.info("[WebSocket] Disconnected gracefully")
        AuditService.log("WEBSOCKET_DISCONNECTED", data: [:])
    }

    // MARK: - Tracking

    /// Subscribes to real-time mempool transactions for a specific Bitcoin address.
    func trackAddress(_ address: String) {
        guard !address.isEmpty else { return }
        trackedAddresses.insert(address)
        logger.info("[WebSocket] Registered address to watch: \(address)")
        AuditService.log("WEBSOCKET_TRACK_ADDRESS", data: ["address": address])

        if isConnected {
            syncTracking()
        } else {
            connect()
        }
    }

    /// Unsubscribes from tracking a specific Bitcoin address on client and server.
    func untrackAddress(_ address: String) {
        trackedAddresses.remove(address)
        logger.info("[WebSocket] Untracked address: \(address)")
        if isConnected {
            syncTracking()
        }
    }

    /// Subscribes to real-time transaction outspend events for a funding txid.
    func trackTx(_ txid: String) {
        guard !txid.isEmpty else { return }
        trackedTxids.insert(txid)
        logger.info("[WebSocket] Registered txid to watch: \(txid)")
        AuditService.log("WEBSOCKET_TRACK_TX", data: ["txid": txid])

        if isConnected {
            syncTracking()
        } else {
            connect()
        }
    }

    /// Unsubscribes from tracking a transaction txid on client and server.
    func untrackTx(_ txid: String) {
        trackedTxids.remove(txid)
        logger.info("[WebSocket] Untracked txid: \(txid)")
        if isConnected {
            syncTracking()
        }
    }

    /// Subscribes to real-time block header announcements.
    func trackBlocks() {
        let payload = """
        { "action": "want", "data": ["blocks"] }
        """
        logger.info("[WebSocket] Requesting block tip stream")
        send(payload)
    }

    // MARK: - Send

    func send(_ text: String) {
        guard isConnected, let webSocketTask else {
            logger.debug("[WebSocket] Outbound message buffered while offline: \(text)")
            pendingOutboundMessages.append(text)
            if pendingOutboundMessages.count > 50 {
                pendingOutboundMessages.removeFirst()
            }
            return
        }
        webSocketTask.send(.string(text)) { [weak self] error in
            guard let self else { return }
            if let error {
                self.logger.error("[WebSocket] Send error: \(error.localizedDescription)")
                AuditService.log("WEBSOCKET_SEND_ERROR", data: ["error": error.localizedDescription])
            } else {
                self.logger.debug("[WebSocket] Frame sent: \(text)")
            }
        }
    }

    private func flushPendingMessages() {
        let messages = pendingOutboundMessages
        pendingOutboundMessages.removeAll()
        for msg in messages {
            send(msg)
        }
    }

    // MARK: - Receive

    private func receiveMessages() {
        webSocketTask?.receive { [weak self] result in
            guard let self else { return }
            Task { @MainActor in
                switch result {
                case .success(let message):
                    switch message {
                    case .string(let text):
                        self.handleMessage(text)
                    case .data(let data):
                        if let text = String(data: data, encoding: .utf8) {
                            self.handleMessage(text)
                        }
                    @unknown default:
                        break
                    }
                    if self.isConnected {
                        self.receiveMessages()
                    }
                case .failure(let error):
                    self.logger.warning("WebSocket connection dropped: \(error.localizedDescription)")
                    self.isConnected = false
                    self.webSocketTask = nil
                    if !self.isManualDisconnect {
                        self.reconnectTask?.cancel()
                        self.reconnectTask = Task.detached { [weak self] in
                            do {
                                try await Task.sleep(nanoseconds: 3_000_000_000)
                                await MainActor.run {
                                    guard let self, !self.isManualDisconnect else { return }
                                    self.connect()
                                }
                            } catch {
                                // Task was cancelled
                            }
                        }
                    }
                }
            }
        }
    }

    // MARK: - Dedup

    private func isRecentlyProcessed(_ txid: String) -> Bool {
        guard let lastSeen = processedTxids[txid] else { return false }
        return Date().timeIntervalSince(lastSeen) < processedTxidTTL
    }

    private func recordProcessedTx(_ txid: String) {
        processedTxids[txid] = Date()
        maybePurgeProcessedTxs()
    }

    /// Purge expired entries at most every 5 minutes.
    /// A time-based purge prevents unbounded growth for low-activity wallets
    /// that never hit the old 50-entry counter threshold.
    private func maybePurgeProcessedTxs() {
        let now = Date()
        if now.timeIntervalSince(lastPurgeTime) < 300 {
            return
        }
        let cutoff = now.timeIntervalSince1970 - processedTxidTTL
        processedTxids = processedTxids.filter { _, date in
            date.timeIntervalSince1970 > cutoff
        }
        lastPurgeTime = now
    }

    // MARK: - Message Handling

    func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8) else { return }

        guard let msg = try? decoder.decode(MempoolWSMessage.self, from: data) else {
            logger.warning("[WebSocket] Failed to decode WS message: \(String(text.prefix(200)))")
            AuditService.log("WEBSOCKET_DECODE_FAILED", data: ["raw": String(text.prefix(200))])
            return
        }

        // 1. Process all transaction payloads (mempool, confirmed, removed)
        let allTxs = aggregateTransactions(from: msg)

        for tx in allTxs {
            let txid = tx.txid
            guard ResilientEsploraClient.isValidTxid(txid) else { continue }

            let matches = matcher.matchAll(
                trackedAddresses: trackedAddresses,
                trackedTxids: trackedTxids,
                msg: msg,
                tx: tx
            )

            for match in matches {
                let targetKey = match.target
                let isTxid = match.isTxid
                let dedupKey = "\(txid)_\(targetKey)"

                if isRecentlyProcessed(dedupKey) { continue }
                recordProcessedTx(dedupKey)

                let amountSats = sumAmount(for: tx, target: targetKey)

                logger.info("Real-time transaction detected via WebSocket for \(targetKey): \(txid)")
                AuditService.log(
                    "WEBSOCKET_MATCH_DETECTED",
                    data: ["target": targetKey, "txid": txid, "amount_sats": "\(amountSats)", "is_txid": "\(isTxid)"]
                )
                onTransactionDetected?(.receive(target: targetKey, txid: txid, amountSats: amountSats))
            }
        }

        // 2. Handle removed transactions (RBF replacements)
        handleRemovedTransactions(msg)

        // 3. Check for block header payload
        if let block = msg.block ?? msg.blocks?.last {
            let height = block.height
            logger.info("Real-time block header received via WebSocket: \(height)")
            AuditService.log("WEBSOCKET_BLOCK_TIP", data: ["height": "\(height)"])
            onBlockHeader?(height)
        }

        // 4. Handle outspends of tracked txids (channel closes)
        if let tracked = msg.trackedTxs {
            for (trackedTxid, trackingInfo) in tracked {
                guard trackedTxids.contains(trackedTxid) else { continue }
                if let outspends = trackingInfo.utxoSpent {
                    for (_, outspend) in outspends {
                        let spendingTxid = outspend.txid
                        let dedupKey = "\(spendingTxid)_outspend_\(trackedTxid)"
                        if isRecentlyProcessed(dedupKey) { continue }
                        recordProcessedTx(dedupKey)

                        logger.info("Outspend detected for tracked txid \(trackedTxid) by \(spendingTxid)")
                        AuditService.log(
                            "WEBSOCKET_TXID_OUTSPENT",
                            data: ["tracked_txid": trackedTxid, "spending_txid": spendingTxid]
                        )
                        onTransactionDetected?(.trackedOutspend(trackedTxid: trackedTxid, spendingTxid: spendingTxid))
                    }
                }
            }
        }
    }

    private func aggregateTransactions(from msg: MempoolWSMessage) -> [MempoolWSTransaction] {
        var allTxs: [MempoolWSTransaction] = []
        allTxs.append(contentsOf: msg.addressTransactions ?? [])
        allTxs.append(contentsOf: msg.blockTransactions ?? [])

        if let multi = msg.multiAddressTransactions {
            for (_, txGroup) in multi {
                allTxs.append(contentsOf: txGroup.mempool ?? [])
                allTxs.append(contentsOf: txGroup.confirmed ?? [])
            }
        }

        return allTxs
    }

    /// RBF-replaced transactions land in the `removed` array.
    /// Fire the callback with amount = 0 so callers can retract a pending payment.
    private func handleRemovedTransactions(_ msg: MempoolWSMessage) {
        if let multi = msg.multiAddressTransactions {
            for (addr, txGroup) in multi {
                guard trackedAddresses.contains(addr) else { continue }
                for tx in txGroup.removed ?? [] {
                    let txid = tx.txid
                    guard ResilientEsploraClient.isValidTxid(txid) else { continue }
                    let dedupKey = "\(txid)_\(addr)"
                    if isRecentlyProcessed(dedupKey) { continue }
                    recordProcessedTx(dedupKey)
                    logger.info("RBF replacement detected for \(addr): \(txid)")
                    AuditService.log("WEBSOCKET_TX_REMOVED", data: ["target": addr, "txid": txid])
                    onTransactionDetected?(.removed(target: addr, txid: txid))
                }
            }
        }
    }

    private func sumAmount(for tx: MempoolWSTransaction, target: String) -> Int64 {
        var amountSats: Int64 = 0
        if let vouts = tx.vout {
            for vout in vouts {
                if vout.scriptpubkeyAddress == target, let val = vout.value {
                    amountSats += val
                }
            }
        }
        return amountSats
    }

    // MARK: - Tracking Sync

    private func syncTracking() {
        let addresses = Array(trackedAddresses)
        if !addresses.isEmpty {
            if let data = try? JSONSerialization.data(withJSONObject: ["track-addresses": addresses]),
               let text = String(data: data, encoding: .utf8) {
                send(text)
            }
        } else {
            send("{ \"track-addresses\": [] }")
        }

        let txids = Array(trackedTxids)
        if !txids.isEmpty {
            if let data = try? JSONSerialization.data(withJSONObject: ["track-txs": txids]),
               let text = String(data: data, encoding: .utf8) {
                send(text)
            }
        } else {
            send("{ \"track-txs\": [] }")
        }
    }
}
