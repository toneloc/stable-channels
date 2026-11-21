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
    /// The channel's value from the node's listing, on the side of the splice the event sees: for a
    /// negotiation the value before it (the listing stays on the old funding until the splice
    /// locks), for a ready event the value on its funding. Nil when the listing does not show that.
    let channelValueSats: UInt64?

    /// Keyed by the funding it resolved to, so each outpoint-less ready on a channel (the open,
    /// then every splice) is stored once instead of all sharing one key. A stale replay still
    /// cannot claim an operation: matching excludes the funding the operation started from.
    var eventKey: String {
        "\(kind.rawValue):\(userChannelId):\(channelId):\(identityTxid ?? fundingTxid ?? "unknown")"
    }

    /// Maps an LDK event against the node's channel listing; nil for anything that is not
    /// SpliceNegotiated or ChannelReady. A stale or early ready event may lack its outpoint, so the
    /// live channel funding fills in.
    init?(event: Event, channels: [ChannelDetails]) {
        switch event {
        case .spliceNegotiated(let channelId, let userChannelId, _, let newFundingTxo):
            self.init(
                kind: .negotiated, userChannelId: userChannelId, channelId: channelId,
                identityTxid: newFundingTxo.txid, funding: newFundingTxo, channels: channels
            )
        case .channelReady(let channelId, let userChannelId, _, let fundingTxo):
            let liveFunding = Self.liveFunding(in: channels, userChannelId: userChannelId, channelId: channelId)
            self.init(
                kind: .ready, userChannelId: userChannelId, channelId: channelId,
                identityTxid: fundingTxo?.txid, funding: fundingTxo ?? liveFunding, channels: channels
            )
        default:
            return nil
        }
    }

    /// Takes `channelValueSats` from the one listed channel with this identity, on the side of the
    /// splice the event sees.
    init(
        kind: Kind,
        userChannelId: String,
        channelId: String,
        identityTxid: String?,
        funding: OutPoint?,
        channels: [ChannelDetails]
    ) {
        let listed = Self.liveChannel(in: channels, userChannelId: userChannelId, channelId: channelId)
        let listedOnFunding = funding != nil && listed?.fundingTxo?.txid == funding?.txid
        let seen = kind == .ready ? listedOnFunding : funding != nil && !listedOnFunding
        self.init(
            kind: kind, userChannelId: userChannelId, channelId: channelId,
            identityTxid: identityTxid, funding: funding,
            channelValueSats: seen ? listed?.channelValueSats : nil
        )
    }

    /// Funding of the one live channel with this identity; nil when absent or ambiguous.
    static func liveFunding(in channels: [ChannelDetails], userChannelId: String, channelId: String) -> OutPoint? {
        liveChannel(in: channels, userChannelId: userChannelId, channelId: channelId)?.fundingTxo
    }

    /// The one live channel with this identity; nil when absent or ambiguous.
    private static func liveChannel(
        in channels: [ChannelDetails], userChannelId: String, channelId: String
    ) -> ChannelDetails? {
        let matches = channels.filter { $0.userChannelId == userChannelId && $0.channelId == channelId }
        return matches.count == 1 ? matches[0] : nil
    }

    init(
        kind: Kind,
        userChannelId: String,
        channelId: String,
        identityTxid: String?,
        funding: OutPoint?,
        channelValueSats: UInt64? = nil
    ) {
        self.kind = kind
        self.userChannelId = userChannelId
        self.channelId = channelId
        self.identityTxid = identityTxid
        fundingTxid = funding?.txid
        fundingVout = funding?.vout
        self.channelValueSats = channelValueSats
    }
}

/// What the notification extension may do with a splice event it cannot fully handle.
enum SpliceEventAcknowledgement {
    /// Background (notification extension) disposition: true means the event is committed, or
    /// safe to drop, and may be acknowledged; false leaves it for the foreground.
    /// A negotiation failure needs the foreground's broadcast-verified bookkeeping, so it is only
    /// acknowledged while no splice is pending.
    ///
    /// `lspSplice` says the splice is known to be the LSP's: the lsp_splice_in wait passes true, the
    /// other waits pass whether the LSP's splice push came within `LspSpliceExpectation`'s window.
    static func backgroundMayAcknowledge(
        _ event: Event,
        db: PaymentDatabase,
        node: LDKNode.Node,
        lspSplice: Bool
    ) -> Bool {
        backgroundMayAcknowledge(event, db: db, lspSplice: lspSplice, channels: node.listChannels)
    }

    /// `channels` is read only for events that need the live channel funding.
    static func backgroundMayAcknowledge(
        _ event: Event,
        db: PaymentDatabase,
        lspSplice: Bool = false,
        channels listChannels: () -> [ChannelDetails]
    ) -> Bool {
        if case .spliceNegotiationFailed = event {
            return db.hasPendingSplice() == false
        }
        guard let receipt = SpliceEventReceipt(event: event, channels: listChannels()) else { return true }
        return db.recordSpliceEvent(receipt, lspSplice: lspSplice)
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
    static let direction = "lsp_splice_in"
    static let notificationType = "lsp_splice"
    static let title = "Channel Update"
    static let body = "Open app to finish updating your channel"
}

/// What a splice event did to the LSP's splice record.
enum LspSpliceChange {
    /// A negotiation no Move claimed was recorded as the LSP's splice.
    case negotiated
    /// The ready event on its funding locked it.
    case locked
    /// The ready event showed the channel shrank: the splice was the waiting splice-out's, which
    /// now has it.
    case wasSpliceOut
}

/// The LSP pushes `lsp_splice_in` before each splice of its own. For a while after that push, a
/// negotiation with no Move of the app's own in flight is taken as the LSP's. The extension and
/// the app both note the push in the shared app-group defaults.
enum LspSpliceExpectation {
    static let windowSecs: TimeInterval = 10 * 60
    static let pushedAtKey = "lsp_splice_pushed_at"

    static func note(_ defaults: UserDefaults?, at now: Date = Date()) {
        defaults?.set(now.timeIntervalSince1970, forKey: pushedAtKey)
    }

    static func isExpected(_ defaults: UserDefaults?, at now: Date = Date()) -> Bool {
        expectedWithin(pushedAt: defaults?.double(forKey: pushedAtKey) ?? 0, now: now.timeIntervalSince1970)
    }

    /// From the push until the window ends; a clock set back before the push expects nothing.
    static func expectedWithin(pushedAt: TimeInterval, now: TimeInterval) -> Bool {
        pushedAt > 0 && now >= pushedAt && now - pushedAt < windowSecs
    }

    /// Whether a push payload is the LSP's splice push; the direction comes as a dictionary or as
    /// JSON text, as the extension reads it.
    static func isSplicePush(_ userInfo: [AnyHashable: Any]) -> Bool {
        if let stability = userInfo["stability"] as? [String: Any] {
            return stability["direction"] as? String == LspSpliceNotice.direction
        }
        guard let text = userInfo["stability"] as? String, let data = text.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return false }
        return json["direction"] as? String == LspSpliceNotice.direction
    }
}

/// Whether a splice negotiated now is expected to be the LSP's: the one rule the app and the
/// extension's handlers share. It is while the LSP's splice push is being handled or within its
/// window (`LspSpliceExpectation`), unless a Move of the user's own is in flight in this process.
/// The extension runs its node only while the app's is stopped, so it has no Move in flight.
///
/// A negotiation names no size, so this cannot tell a splice-out from the LSP's splice. The
/// recorder keeps such a splice-out with the LSP's splice and settles it once the channel is
/// ready, by which way its value moved (`SpliceEventRecorder.recordLspSpliceSQL`).
enum SpliceOwnership {
    static func expectsLspSplice(
        _ defaults: UserDefaults?,
        handlingSplicePush: Bool = false,
        moveInFlight: Bool = false,
        at now: Date = Date()
    ) -> Bool {
        !moveInFlight && (handlingSplicePush || LspSpliceExpectation.isExpected(defaults, at: now))
    }
}
