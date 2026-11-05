import Foundation

/// How a block header from the WebSocket feed relates to the stored header chain.
/// The feed is unauthenticated, so comparisons never overflow and a header far ahead of the
/// HTTP-verified tip is not stored.
enum HeaderChainPolicy {
    /// How many blocks a WebSocket header may run ahead of the HTTP-verified tip and still be stored.
    static let maxLeadOverVerifiedTip: UInt32 = 6

    enum Step: Equatable {
        /// The header links to the stored tip.
        case append
        /// Blocks were missed while the app was away.
        case catchUp
        /// A competing block at the same or a lower height.
        case reorg
    }

    static func step(tipHeight: UInt32, tipHash: String, incomingHeight: UInt32, incomingPrevHash: String) -> Step {
        if incomingPrevHash == tipHash { return .append }
        // `tipHeight + 1` would trap on a stored maximum height, so compare without adding.
        if incomingHeight > tipHeight, incomingHeight - tipHeight > 1 { return .catchUp }
        return .reorg
    }

    /// Whether a header may be stored. Before the first verified tip there is nothing to compare against.
    static func canStore(incomingHeight: UInt32, verifiedTip: UInt32) -> Bool {
        guard verifiedTip > 0, incomingHeight > verifiedTip else { return true }
        return incomingHeight - verifiedTip <= maxLeadOverVerifiedTip
    }
}
