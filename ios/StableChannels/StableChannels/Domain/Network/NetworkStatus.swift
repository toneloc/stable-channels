import Foundation

/// Value object representing network connectivity status.
public enum NetworkStatus: Equatable, Sendable {
    case online
    case offline

    public var isConnected: Bool {
        self == .online
    }
}
