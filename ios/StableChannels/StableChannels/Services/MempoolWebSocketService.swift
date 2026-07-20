import Foundation
import os.log

/// Manages a native Swift `URLSessionWebSocketTask` connection to Mempool.space
/// for real-time sub-second incoming payment alerts, txid resolution, and block tip updates.
@MainActor
final class MempoolWebSocketService {
    private(set) var isConnected: Bool = false
    private let wsEndpointURL: URL
    private let logger = Logger(subsystem: "com.stablechannels", category: "websocket")

    private var urlSession: URLSession?
    private var webSocketTask: URLSessionWebSocketTask?
    private var trackedAddresses: Set<String> = []
    private var trackedTxids: Set<String> = []
    private var pendingOutboundMessages: [String] = []
    private var isManualDisconnect: Bool = false

    /// Fired when a transaction is detected hitting a tracked address or txid outspend.
    var onTransactionDetected: ((_ addressOrTxid: String, _ txid: String, _ amountSats: Int64) -> Void)?

    /// Fired when a new block header is mined.
    var onBlockHeader: ((_ height: UInt32) -> Void)?

    init(endpointURL: URL = URL(string: "wss://mempool.space/api/v1/ws")!) {
        self.wsEndpointURL = endpointURL
    }

    /// Establishes the WebSocket connection and starts the message listener loop.
    func connect() {
        guard !isConnected else { return }
        isManualDisconnect = false

        // Invalidate old session to prevent resource leaks
        urlSession?.invalidateAndCancel()
        let config = URLSessionConfiguration.default
        let session = URLSession(configuration: config)
        self.urlSession = session

        webSocketTask = session.webSocketTask(with: wsEndpointURL)
        webSocketTask?.resume()
        isConnected = true

        logger.info("[WebSocket] Connected to Mempool WebSocket at \(self.wsEndpointURL.absoluteString)")
        AuditService.log("WEBSOCKET_CONNECTED", data: ["url": wsEndpointURL.absoluteString])

        // Re-subscribe to any previously tracked addresses and txids
        for address in trackedAddresses {
            sendTrackAddress(address)
        }
        for txid in trackedTxids {
            sendTrackTx(txid)
        }

        // Subscribe to real-time block header updates
        trackBlocks()

        // Flush any pending outbound messages buffered while disconnected
        flushPendingMessages()

        // Start async listening loop
        receiveMessages()
    }

    /// Disconnects the WebSocket gracefully and invalidates the session.
    func disconnect() {
        isManualDisconnect = true
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
            sendTrackAddress(address)
        } else {
            connect()
        }
    }

    /// Unsubscribes from tracking a specific Bitcoin address on client and server.
    func untrackAddress(_ address: String) {
        trackedAddresses.remove(address)
        logger.info("[WebSocket] Untracked address: \(address)")
        if isConnected {
            send("""
            { "untrack-address": "\(address)" }
            """)
        }
    }

    /// Subscribes to real-time transaction outspend events for a funding txid.
    func trackTx(_ txid: String) {
        guard !txid.isEmpty else { return }
        trackedTxids.insert(txid)
        logger.info("[WebSocket] Registered txid to watch: \(txid)")
        AuditService.log("WEBSOCKET_TRACK_TX", data: ["txid": txid])
        if isConnected {
            sendTrackTx(txid)
        } else {
            connect()
        }
    }

    /// Unsubscribes from tracking a transaction txid on client and server.
    func untrackTx(_ txid: String) {
        trackedTxids.remove(txid)
        logger.info("[WebSocket] Untracked txid: \(txid)")
        if isConnected {
            send("""
            { "untrack-tx": "\(txid)" }
            """)
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

    private func sendTrackAddress(_ address: String) {
        let payload = """
        { "track-address": "\(address)" }
        """
        send(payload)
    }

    private func sendTrackTx(_ txid: String) {
        let payload = """
        { "track-tx": "\(txid)" }
        """
        send(payload)
    }

    private func send(_ text: String) {
        guard isConnected, let webSocketTask else {
            logger.debug("[WebSocket] Outbound message buffered while offline: \(text)")
            pendingOutboundMessages.append(text)
            return
        }
        webSocketTask.send(.string(text)) { [weak self] error in
            if let error {
                Task { @MainActor [weak self] in
                    self?.logger.error("[WebSocket] Send error: \(error.localizedDescription)")
                    AuditService.log("WEBSOCKET_SEND_ERROR", data: ["error": error.localizedDescription])
                }
            } else {
                Task { @MainActor [weak self] in
                    self?.logger.debug("[WebSocket] Frame sent: \(text)")
                }
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
            Task { @MainActor [weak self] in
                guard let self else { return }
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
                    // Auto-reconnect if not manually disconnected
                    if !self.isManualDisconnect {
                        try? await Task.sleep(nanoseconds: 3_000_000_000)
                        self.connect()
                    }
                }
            }
        }
    }

    private func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }

        // 1. Check for address-transactions payload
        if let addressTxs = json["address-transactions"] as? [[String: Any]],
           let firstTx = addressTxs.first,
           let txid = firstTx["txid"] as? String,
           ResilientEsploraClient.isValidTxid(txid) {
            let targetKey = findMatchingTarget(json: json, firstTx: firstTx)
            if let targetKey {
                var amountSats: Int64 = 0
                if let vouts = firstTx["vout"] as? [[String: Any]] {
                    for vout in vouts {
                        if let addr = vout["scriptpubkey_address"] as? String, addr == targetKey,
                           let val = vout["value"] as? NSNumber {
                            amountSats += val.int64Value
                        }
                    }
                }
                logger.info("Real-time transaction detected via WebSocket for \(targetKey): \(txid)")
                AuditService.log(
                    "WEBSOCKET_MATCH_DETECTED",
                    data: ["target": targetKey, "txid": txid, "amount_sats": "\(amountSats)"]
                )
                onTransactionDetected?(targetKey, txid, amountSats)
            }
        }

        // 2. Check for block header payload
        if let block = json["block"] as? [String: Any],
           let heightNum = block["height"] as? NSNumber {
            let height = heightNum.uint32Value
            logger.info("Real-time block header received via WebSocket: \(height)")
            AuditService.log("WEBSOCKET_BLOCK_TIP", data: ["height": "\(height)"])
            onBlockHeader?(height)
        }
    }

    private func findMatchingTarget(json: [String: Any], firstTx: [String: Any]) -> String? {
        // Direct address in response JSON
        if let respAddr = json["address"] as? String, trackedAddresses.contains(respAddr) {
            return respAddr
        }
        // Match output scriptpubkey_address
        if let vouts = firstTx["vout"] as? [[String: Any]] {
            for vout in vouts {
                if let addr = vout["scriptpubkey_address"] as? String, trackedAddresses.contains(addr) {
                    return addr
                }
            }
        }
        // Match input prevout scriptpubkey_address
        if let vins = firstTx["vin"] as? [[String: Any]] {
            for vin in vins {
                if let prevout = vin["prevout"] as? [String: Any],
                   let addr = prevout["scriptpubkey_address"] as? String,
                   trackedAddresses.contains(addr) {
                    return addr
                }
                if let inputTxid = vin["txid"] as? String, trackedTxids.contains(inputTxid) {
                    return inputTxid
                }
            }
        }
        // Match tracked txids directly
        if let respTxid = json["txid"] as? String, trackedTxids.contains(respTxid) {
            return respTxid
        }
        return nil
    }
}
