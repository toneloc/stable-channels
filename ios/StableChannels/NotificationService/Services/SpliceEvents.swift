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
