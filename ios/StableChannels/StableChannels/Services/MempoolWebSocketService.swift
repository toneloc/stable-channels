import Foundation
import os.log

/// Manages a native Swift `URLSessionWebSocketTask` connection to Mempool.space
/// for real-time sub-second incoming payment alerts, txid resolution, and block tip updates.
@Observable
@MainActor
final class MempoolWebSocketService {
    private(set) var isConnected: Bool = false
    private let wsEndpointURL: URL
    private let logger = Logger(subsystem: "com.stablechannels", category: "websocket")

    private var webSocketTask: URLSessionWebSocketTask?
    private var trackedAddresses: Set<String> = []
    private var trackedTxids: Set<String> = []
    private var isManualDisconnect: Bool = false

    /// Fired when a transaction is detected hitting a tracked address or txid outspend.
    var onTransactionDetected: ((_ addressOrTxid: String, _ txid: String) -> Void)?

    /// Fired when a new block header is mined.
    var onBlockHeader: ((_ height: UInt32) -> Void)?

    init(endpointURL: URL = URL(string: "wss://mempool.space/api/v1/ws")!) {
        self.wsEndpointURL = endpointURL
    }

    /// Establishes the WebSocket connection and starts the message listener loop.
    func connect() {
        guard !isConnected else { return }
        isManualDisconnect = false

        let session = URLSession(configuration: .default)
        webSocketTask = session.webSocketTask(with: wsEndpointURL)
        webSocketTask?.resume()
        isConnected = true

        logger.info("Connected to Mempool WebSocket at \(self.wsEndpointURL.absoluteString)")

        // Re-subscribe to any previously tracked addresses and txids
        for address in trackedAddresses {
            sendTrackAddress(address)
        }
        for txid in trackedTxids {
            sendTrackTx(txid)
        }

        // Subscribe to real-time block header updates
        trackBlocks()

        // Start async listening loop
        receiveMessages()
    }

    /// Disconnects the WebSocket gracefully.
    func disconnect() {
        isManualDisconnect = true
        webSocketTask?.cancel(with: .goingAway, reason: nil)
        webSocketTask = nil
        isConnected = false
        logger.info("Mempool WebSocket disconnected")
    }

    /// Subscribes to real-time mempool transactions for a specific Bitcoin address.
    func trackAddress(_ address: String) {
        guard !address.isEmpty else { return }
        trackedAddresses.insert(address)
        if isConnected {
            sendTrackAddress(address)
        } else {
            connect()
        }
    }

    /// Unsubscribes from tracking a specific Bitcoin address.
    func untrackAddress(_ address: String) {
        trackedAddresses.remove(address)
    }

    /// Subscribes to real-time transaction outspend events for a funding txid.
    func trackTx(_ txid: String) {
        guard !txid.isEmpty else { return }
        trackedTxids.insert(txid)
        if isConnected {
            sendTrackTx(txid)
        } else {
            connect()
        }
    }

    /// Unsubscribes from tracking a transaction txid.
    func untrackTx(_ txid: String) {
        trackedTxids.remove(txid)
    }

    /// Subscribes to real-time block header announcements.
    func trackBlocks() {
        guard isConnected else { return }
        let payload = """
        { "action": "want", "data": ["blocks"] }
        """
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
        webSocketTask?.send(.string(text)) { [weak self] error in
            if let error {
                Task { @MainActor [weak self] in
                    self?.logger.error("WebSocket send error: \(error.localizedDescription)")
                }
            }
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
            // Find which tracked address this matches
            for address in trackedAddresses {
                onTransactionDetected?(address, txid)
            }
            logger.info("Real-time transaction detected via WebSocket: \(txid)")
        }

        // 2. Check for block header payload
        if let block = json["block"] as? [String: Any],
           let heightNum = block["height"] as? NSNumber {
            let height = heightNum.uint32Value
            onBlockHeader?(height)
            logger.info("Real-time block header received via WebSocket: \(height)")
        }
    }
}
