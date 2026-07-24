import Foundation
import os.log

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

struct MempoolWSPrevout: Decodable {
    let scriptpubkeyAddress: String?

    enum CodingKeys: String, CodingKey {
        case scriptpubkeyAddress = "scriptpubkey_address"
    }
}

struct MempoolWSVin: Decodable {
    let txid: String?
    let prevout: MempoolWSPrevout?
}

struct MempoolWSTransaction: Decodable {
    let txid: String
    let vout: [MempoolWSVout]?
    let vin: [MempoolWSVin]?
}

struct MempoolWSMessage: Decodable {
    let block: MempoolWSBlock?
    let addressTransactions: [MempoolWSTransaction]?
    let address: String?
    let txid: String?

    enum CodingKeys: String, CodingKey {
        case block
        case addressTransactions = "address-transactions"
        case address
        case txid
    }
}

/// Manages a native Swift `URLSessionWebSocketTask` connection to Mempool.space
/// for real-time sub-second incoming payment alerts, txid resolution, and block tip updates.
@MainActor
final class MempoolWebSocketService: NSObject, URLSessionWebSocketDelegate {
    private(set) var isConnected: Bool = false
    private let wsEndpointURL: URL
    private let logger = Logger(subsystem: "com.stablechannels", category: "websocket")
    private let decoder = JSONDecoder()

    private var urlSession: URLSession?
    private var webSocketTask: URLSessionWebSocketTask?
    private var trackedAddresses: Set<String> = []
    private var trackedTxids: Set<String> = []
    private var pendingOutboundMessages: [String] = []
    private var processedTxids: [String: Date] = [:]
    private let processedTxidTTL: TimeInterval = 900 // 15 minutes
    private var processedTxidPurgeCounter: Int = 0
    private var isManualDisconnect: Bool = false
    private var reconnectTask: Task<Void, Never>?

    /// Fired when a transaction is detected hitting a tracked address or txid outspend.
    var onTransactionDetected: ((_ target: String, _ isTxid: Bool, _ txid: String, _ amountSats: Int64) -> Void)?

    /// Fired when a new block header is mined.
    var onBlockHeader: ((_ height: UInt32) -> Void)?

    init(endpointURL: URL = URL(string: "wss://mempool.space/api/v1/ws")!) {
        self.wsEndpointURL = endpointURL
        super.init()
    }

    /// Establishes the WebSocket connection and starts the message listener loop.
    func connect() {
        guard !isConnected else { return }

        // Cancel any existing task and reconnect loop before creating a new one
        reconnectTask?.cancel()
        reconnectTask = nil
        webSocketTask?.cancel(with: .goingAway, reason: nil)
        webSocketTask = nil
        isManualDisconnect = false

        // Invalidate old session to prevent resource leaks
        urlSession?.invalidateAndCancel()
        let config = URLSessionConfiguration.default
        let session = URLSession(configuration: config, delegate: self, delegateQueue: nil)
        self.urlSession = session

        webSocketTask = session.webSocketTask(with: wsEndpointURL)
        webSocketTask?.resume()

        logger.info("[WebSocket] Initiated connection to \(self.wsEndpointURL.absoluteString)")
        // Note: isConnected is set to true in urlSession(_:didOpenWithProtocol:)
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

            // Re-subscribe to any previously tracked addresses and txids
            self.syncTracking()

            // Subscribe to real-time block header updates
            self.trackBlocks()

            // Flush any pending outbound messages buffered while disconnected
            self.flushPendingMessages()

            // Start async listening loop
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

    func send(_ text: String) {
        guard isConnected, let webSocketTask else {
            logger.debug("[WebSocket] Outbound message buffered while offline: \(text)")
            pendingOutboundMessages.append(text)
            // Cap pending queue at 50 entries — drop oldest if exceeded
            if pendingOutboundMessages.count > 50 {
                pendingOutboundMessages.removeFirst()
            }
            return
        }
        webSocketTask.send(.string(text)) { error in
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
                    // Auto-reconnect off MainActor to avoid freezing UI
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

    private func isRecentlyProcessed(_ txid: String) -> Bool {
        guard let lastSeen = processedTxids[txid] else { return false }
        return Date().timeIntervalSince(lastSeen) < processedTxidTTL
    }

    private func recordProcessedTx(_ txid: String) {
        processedTxids[txid] = Date()
        processedTxidPurgeCounter += 1
        if processedTxidPurgeCounter >= 50 {
            let cutoff = Date().timeIntervalSince1970 - processedTxidTTL
            processedTxids = processedTxids.filter { _, date in
                date.timeIntervalSince1970 > cutoff
            }
            processedTxidPurgeCounter = 0
        }
    }

    func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8) else { return }

        guard let msg = try? decoder.decode(MempoolWSMessage.self, from: data) else {
            logger.warning("[WebSocket] Failed to decode WS message: \(String(text.prefix(200)))")
            AuditService.log("WEBSOCKET_DECODE_FAILED", data: ["raw": String(text.prefix(200))])
            return
        }

        // 1. Check for address-transactions payload
        if let addressTxs = msg.addressTransactions,
           let firstTx = addressTxs.first,
           ResilientEsploraClient.isValidTxid(firstTx.txid) {
            let txid = firstTx.txid
            if isRecentlyProcessed(txid) { return }
            recordProcessedTx(txid)

            let targetMatch = findMatchingTarget(msg: msg, firstTx: firstTx)
            if let targetKey = targetMatch.0 {
                let isTxid = targetMatch.1
                var amountSats: Int64 = 0
                if let vouts = firstTx.vout {
                    for vout in vouts {
                        if vout.scriptpubkeyAddress == targetKey, let val = vout.value {
                            amountSats += val
                        }
                    }
                }
                logger.info("Real-time transaction detected via WebSocket for \(targetKey): \(txid)")
                AuditService.log(
                    "WEBSOCKET_MATCH_DETECTED",
                    data: ["target": targetKey, "txid": txid, "amount_sats": "\(amountSats)", "is_txid": "\(isTxid)"]
                )
                onTransactionDetected?(targetKey, isTxid, txid, amountSats)
            }
        }

        // 2. Check for block header payload
        if let block = msg.block {
            let height = block.height
            logger.info("Real-time block header received via WebSocket: \(height)")
            AuditService.log("WEBSOCKET_BLOCK_TIP", data: ["height": "\(height)"])
            onBlockHeader?(height)
        }
    }

    func findMatchingTarget(msg: MempoolWSMessage, firstTx: MempoolWSTransaction) -> (String?, Bool) {
        // Direct address in response JSON
        if let respAddr = msg.address, trackedAddresses.contains(respAddr) {
            return (respAddr, false)
        }
        // Match output scriptpubkey_address
        if let vouts = firstTx.vout {
            for vout in vouts {
                if let addr = vout.scriptpubkeyAddress, trackedAddresses.contains(addr) {
                    return (addr, false)
                }
            }
        }
        // Match input prevout scriptpubkey_address
        if let vins = firstTx.vin {
            for vin in vins {
                if let addr = vin.prevout?.scriptpubkeyAddress, trackedAddresses.contains(addr) {
                    return (addr, false)
                }
                if let inputTxid = vin.txid, trackedTxids.contains(inputTxid) {
                    return (inputTxid, true)
                }
            }
        }
        // Match tracked txids directly
        if let respTxid = msg.txid, trackedTxids.contains(respTxid) {
            return (respTxid, true)
        }
        return (nil, false)
    }
}
