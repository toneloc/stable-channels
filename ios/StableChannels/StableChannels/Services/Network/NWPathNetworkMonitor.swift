import Foundation
import Network

/// Concrete infrastructure implementation of NetworkMonitoring using Apple's Network framework.
public final class NWPathNetworkMonitor: NetworkMonitoring, @unchecked Sendable {
    public static let shared = NWPathNetworkMonitor()

    private let monitor: NWPathMonitor
    private let monitorQueue = DispatchQueue(label: "com.stablechannels.networkmonitor", qos: .utility)
    private let lock = NSLock()
    private var internalStatus: NetworkStatus = .online
    private var isStarted = false
    public var onStatusChange: (@Sendable (NetworkStatus) -> Void)?

    public var currentStatus: NetworkStatus {
        lock.lock()
        defer { lock.unlock() }
        return internalStatus
    }

    public var isOnline: Bool {
        currentStatus.isConnected
    }

    public init(monitor: NWPathMonitor = NWPathMonitor()) {
        self.monitor = monitor
    }

    deinit {
        stop()
    }

    public func start() {
        lock.lock()
        guard !isStarted else {
            lock.unlock()
            return
        }
        isStarted = true
        lock.unlock()

        monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            let newStatus: NetworkStatus = (path.status == .satisfied) ? .online : .offline
            self.lock.lock()
            let changed = (self.internalStatus != newStatus)
            self.internalStatus = newStatus
            let handler = self.onStatusChange
            self.lock.unlock()

            if changed {
                AuditService.log("NETWORK_REACHABILITY_CHANGED", data: [
                    "status": (newStatus == .online) ? "online" : "offline",
                    "expensive": "\(path.isExpensive)",
                    "constrained": "\(path.isConstrained)"
                ])
                handler?(newStatus)
            }
        }

        monitor.start(queue: monitorQueue)
    }

    public func stop() {
        lock.lock()
        guard isStarted else {
            lock.unlock()
            return
        }
        isStarted = false
        lock.unlock()

        monitor.cancel()
    }
}
