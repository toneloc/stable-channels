import Foundation
import LDKNode

struct SpliceChannelIdentity: Equatable {
    let userChannelId: String
    let channelId: String
    let previousFundingTxid: String?
}

/// A splice event reduced to what both processes persist before acknowledging it to LDK.
struct SpliceEventReceipt: Equatable {
    enum Kind: String {
        case negotiated
        case ready
    }

    let kind: Kind
    let userChannelId: String
    let channelId: String
    /// Txid carried by the event itself.
    let identityTxid: String?
    let fundingTxid: String?
    let fundingVout: UInt32?

    /// Keyed by the funding it resolved to, so each outpoint-less ready on a channel (the open,
    /// then every splice) is stored once instead of all sharing one key. A stale replay still
    /// cannot claim an operation: matching excludes the funding the operation started from.
    var eventKey: String {
        "\(kind.rawValue):\(userChannelId):\(channelId):\(identityTxid ?? fundingTxid ?? "unknown")"
    }

    /// Maps an LDK event; nil for anything that is not SpliceNegotiated or ChannelReady.
    /// A stale or early ready event may lack its outpoint, so the live channel funding fills in.
    init?(event: Event, liveFunding: (_ userChannelId: String, _ channelId: String) -> OutPoint?) {
        switch event {
        case .spliceNegotiated(let channelId, let userChannelId, _, let newFundingTxo):
            self.init(
                kind: .negotiated, userChannelId: userChannelId, channelId: channelId,
                identityTxid: newFundingTxo.txid, funding: newFundingTxo
            )
        case .channelReady(let channelId, let userChannelId, _, let fundingTxo):
            self.init(
                kind: .ready, userChannelId: userChannelId, channelId: channelId,
                identityTxid: fundingTxo?.txid, funding: fundingTxo ?? liveFunding(userChannelId, channelId)
            )
        default:
            return nil
        }
    }

    /// Funding of the one live channel with this identity; nil when absent or ambiguous.
    static func liveFunding(in channels: [ChannelDetails], userChannelId: String, channelId: String) -> OutPoint? {
        let matches = channels.filter { $0.userChannelId == userChannelId && $0.channelId == channelId }
        return matches.count == 1 ? matches[0].fundingTxo : nil
    }

    init(
        kind: Kind,
        userChannelId: String,
        channelId: String,
        identityTxid: String?,
        funding: OutPoint?
    ) {
        self.kind = kind
        self.userChannelId = userChannelId
        self.channelId = channelId
        self.identityTxid = identityTxid
        fundingTxid = funding?.txid
        fundingVout = funding?.vout
    }
}

/// What the notification extension may do with a splice event it cannot fully handle.
enum SpliceEventAcknowledgement {
    /// Background (notification extension) disposition: true means the event is committed, or
    /// safe to drop, and may be acknowledged; false leaves it for the foreground.
    /// A negotiation failure needs the foreground's broadcast-verified bookkeeping, so it is only
    /// acknowledged while no splice is pending.
    static func backgroundMayAcknowledge(_ event: Event, db: PaymentDatabase, node: LDKNode.Node) -> Bool {
        backgroundMayAcknowledge(event, db: db, channels: node.listChannels)
    }

    /// `channels` is read only for events that need the live channel funding.
    static func backgroundMayAcknowledge(
        _ event: Event,
        db: PaymentDatabase,
        channels listChannels: () -> [ChannelDetails]
    ) -> Bool {
        if case .spliceNegotiationFailed = event {
            return db.hasPendingSplice() == false
        }
        let channels = listChannels()
        let receipt = SpliceEventReceipt(event: event) {
            SpliceEventReceipt.liveFunding(in: channels, userChannelId: $0, channelId: $1)
        }
        guard let receipt else { return true }
        return db.recordSpliceEvent(receipt)
    }
}

/// When the extension's wait for an LSP splice (`lsp_splice_in`) ends: at a failed negotiation, at
/// the lock plus a settle window for a stability payment that may follow it, or at the deadline.
/// Only events on the channel the wait started from count. A ready event locks the splice once that
/// channel's funding moved off the one the wait started from, or when it is on a negotiation seen in
/// this wait (a splice that locked before the wait began); a replayed ready for the current funding
/// with no such negotiation does not end the wait.
struct LspSpliceWait {
    enum Outcome: String {
        case timedOut = "timed_out"
        case negotiated
        case locked
        case failed
    }

    let userChannelId: String?
    let startFundingTxid: String?
    let deadline: Date
    let settleSecs: TimeInterval
    private(set) var negotiatedTxids: Set<String> = []
    private(set) var lockedAt: Date?
    private(set) var failed = false

    init(userChannelId: String?, startFundingTxid: String?, deadline: Date, settleSecs: TimeInterval) {
        self.userChannelId = userChannelId
        self.startFundingTxid = startFundingTxid
        self.deadline = deadline
        self.settleSecs = settleSecs
    }

    /// The channel the wait watches: the saved channel when the node lists it, else the node's only
    /// channel; nil when neither identifies one.
    static func channel(in channels: [ChannelDetails], savedUserChannelId: String?) -> ChannelDetails? {
        if let savedUserChannelId, let saved = channels.first(where: { $0.userChannelId == savedUserChannelId }) {
            return saved
        }
        return channels.count == 1 ? channels[0] : nil
    }

    /// A lock is the outcome once seen; a failure after it belongs to some later negotiation.
    var outcome: Outcome {
        if lockedAt != nil { return .locked }
        if failed { return .failed }
        return negotiatedTxids.isEmpty ? .timedOut : .negotiated
    }

    func isDone(at now: Date) -> Bool {
        if failed || now >= deadline { return true }
        guard let lockedAt else { return false }
        return now >= lockedAt.addingTimeInterval(settleSecs)
    }

    /// `liveFunding` gives the channel's current funding txid, for a ready event without one.
    mutating func see(_ event: Event, at now: Date, liveFunding: (_ userChannelId: String) -> String?) {
        switch event {
        case .spliceNegotiated(_, let eventChannel, _, let newFundingTxo):
            if watches(eventChannel) { negotiatedTxids.insert(newFundingTxo.txid) }
        case .channelReady(_, let eventChannel, _, let fundingTxo):
            guard watches(eventChannel), lockedAt == nil,
                  let funding = fundingTxo?.txid ?? liveFunding(eventChannel) else { return }
            let moved = startFundingTxid.map { funding != $0 } ?? false
            guard moved || negotiatedTxids.contains(funding) else { return }
            lockedAt = now
        case .spliceNegotiationFailed(_, let eventChannel, _):
            if watches(eventChannel) { failed = true }
        default:
            break
        }
    }

    private func watches(_ eventChannel: String) -> Bool {
        userChannelId != nil && eventChannel == userChannelId
    }
}

/// What the user sees for an LSP splice push (`lsp_splice_in`) the extension could not see through to the lock.
/// The app is open for the splice in the foreground, so it shows no banner there.
enum LspSpliceNotice {
    static let notificationType = "lsp_splice"
    static let title = "Channel Update"
    static let body = "Open app to finish updating your channel"
}
