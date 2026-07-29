import Foundation
import os.log

/// Manages WebSocket reconnection with exponential backoff and a keepalive ping timer.
@MainActor
final class ReconnectionManager: NSObject {
    var reconnectAttempts: Int = 0
    let maxReconnectDelay: UInt64 // seconds
    private var reconnectTask: Task<Void, Never>?
    private var pingTimer: Timer?
    var isManualDisconnect: Bool = false

    /// Called every 30s — the service should send a WebSocket ping frame.
    var onPingRequest: (() -> Void)?

    /// Called when the backoff delay expires — the service should call `connect()`.
    var onReconnect: (() -> Void)?

    private let logger = Logger(subsystem: "com.stablechannels", category: "reconnect")

    init(maxReconnectDelay: UInt64 = 60) {
        self.maxReconnectDelay = maxReconnectDelay
    }

    // MARK: - Lifecycle

    /// Connection is alive — reset attempts and start the ping timer.
    func connected() {
        reconnectAttempts = 0
        stopReconnectTask()
        startPingTimer()
    }

    /// Connection dropped unexpectedly — stop ping and schedule reconnect.
    func disconnected() {
        stopPingTimer()
        if isManualDisconnect { return }
        scheduleReconnect()
    }

    /// Graceful shutdown — stop everything.
    func stop() {
        isManualDisconnect = true
        stopReconnectTask()
        stopPingTimer()
    }

    /// Reset manual-disconnect flag before a fresh `connect()`.
    func reset() {
        isManualDisconnect = false
        reconnectAttempts = 0
    }

    // MARK: - Ping Timer

    private func startPingTimer() {
        stopPingTimer()
        pingTimer = Timer.scheduledTimer(withTimeInterval: 30.0, repeats: true) { [weak self] _ in
            guard let self else { return }
            self.onPingRequest?()
        }
    }

    private func stopPingTimer() {
        pingTimer?.invalidate()
        pingTimer = nil
    }

    // MARK: - Reconnection Backoff

    private func scheduleReconnect() {
        stopReconnectTask()

        let delay = min(UInt64(pow(2.0, Double(reconnectAttempts))), maxReconnectDelay)
        reconnectAttempts += 1

        logger.info("Scheduling reconnect in \(delay)s (attempt \(self.reconnectAttempts))")

        reconnectTask = Task { [weak self] in
            do {
                try await Task.sleep(nanoseconds: delay * 1_000_000_000)
                await MainActor.run {
                    guard let self, !self.isManualDisconnect else { return }
                    self.onReconnect?()
                }
            } catch {
                // Task was cancelled
            }
        }
    }

    func stopReconnectTask() {
        reconnectTask?.cancel()
        reconnectTask = nil
    }
}
