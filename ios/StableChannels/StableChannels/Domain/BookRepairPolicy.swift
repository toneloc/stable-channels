import Foundation

/// Preconditions defining whether a channel's books can safely be repaired.
enum BookRepairPolicy {
    /// Evaluates preconditions before querying status providers.
    static func canAttemptRepair(
        hasUserChannelId: Bool,
        hasReadyChannel: Bool,
        isChannelClosing: Bool,
        isSweeping: Bool,
        hasPendingSpliceInMemory: Bool,
        price: Double
    ) -> Bool {
        guard hasUserChannelId, hasReadyChannel else { return false }
        guard !isChannelClosing, !isSweeping, !hasPendingSpliceInMemory else { return false }
        guard price > 0.0 else { return false }
        return true
    }
}
