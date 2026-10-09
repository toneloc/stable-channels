import Foundation

/// Contract defining network reachability monitoring capabilities.
public protocol NetworkMonitoring: AnyObject, Sendable {
    var currentStatus: NetworkStatus { get }
    var isOnline: Bool { get }
    var onStatusChange: (@Sendable (NetworkStatus) -> Void)? { get set }
    func start()
    func stop()
}
