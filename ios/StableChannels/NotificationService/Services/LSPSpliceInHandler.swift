import Foundation
import UserNotifications
import LDKNode

/// Handler for "lsp_splice_in": the LSP is about to splice its own funds into the channel. Both
/// sides sign a splice, so the node stays up until the splice locks (plus a short settle for a
/// stability payment that may follow), fails, or the run's deadline. Splice events are recorded as
/// in the other handlers, and a payment that arrives meanwhile is handled as in the lsp_to_user wait.
final class LSPSpliceInHandler: PaymentHandler {
    var direction: PaymentDirection { .lspSpliceIn }

    /// After the lock, how long the node stays up for a stability payment the LSP may send next.
    static let settleSecs: TimeInterval = 3
    /// The wait's length when the run gives no deadline, as the lsp_to_user wait.
    static let defaultWaitSecs: TimeInterval = 22

    private let window: StabilitySyncGate?
    private let channelsAtStart: [ChannelDetails]?

    /// `window` carries the run's deadline and whether the extension already expired or finished.
    /// `channelsAtStart` lists the channels as the node started, before it connected to the LSP, so
    /// a splice that locks before the wait begins is not taken for the starting funding.
    init(window: StabilitySyncGate? = nil, channelsAtStart: [ChannelDetails]? = nil) {
        self.window = window
        self.channelsAtStart = channelsAtStart
    }

    func handle(
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher,
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool?) -> Void
    ) {
        let channel = LspSpliceWait.channel(
            in: channelsAtStart ?? node.listChannels(), savedUserChannelId: db.activeUserChannelId()
        )
        var wait = LspSpliceWait(
            userChannelId: channel?.userChannelId,
            startFundingTxid: channel?.fundingTxo?.txid,
            deadline: window?.deadline ?? Date().addingTimeInterval(Self.defaultWaitSecs),
            settleSecs: Self.settleSecs
        )
        var receipts = LSPWakeReceipts()
        var paymentLeftForForeground = false

        eventLoop: while !wait.isDone(at: Date()), window?.isCancelled() != true {
            guard let event = node.nextEvent() else {
                Thread.sleep(forTimeInterval: 0.5)
                continue
            }
            switch event {
            case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
                switch receipts.receive(
                    paymentId: paymentId,
                    paymentHash: paymentHash,
                    amountMsat: amountMsat,
                    customRecords: customRecords,
                    node: node,
                    db: db,
                    priceFetcher: priceFetcher
                ) {
                case .next, .controlHandled:
                    break
                case .stop:
                    paymentLeftForForeground = true
                    break eventLoop
                }
            case .spliceNegotiated, .channelReady, .spliceNegotiationFailed:
                // Commit splice details before acking; a failure while a splice is pending is
                // left for the foreground.
                guard SpliceEventAcknowledgement.backgroundMayAcknowledge(
                    event, db: db, node: node,
                    lspSplice: SpliceOwnership.expectsLspSplice(
                        UserDefaults(suiteName: Constants.appGroup), handlingSplicePush: true
                    )
                ) else { break eventLoop }
                try? node.eventHandled()
                wait.see(event, at: Date()) { userChannelId in
                    node.listChannels().first { $0.userChannelId == userChannelId }?.fundingTxo?.txid
                }
            case .paymentSuccessful, .paymentFailed:
                // Not acked on a failed write: the foreground replays and records it.
                guard recordOutgoingOutcome(event, node: node, db: db) else { break eventLoop }
                try? node.eventHandled()
            default:
                try? node.eventHandled()
            }
        }
        window?.log("lsp_splice_in wait ended: outcome=\(wait.outcome.rawValue)")

        let result = Self.content(
            wait: wait, receipts: receipts, paymentLeftForForeground: paymentLeftForForeground,
            base: baseContent, mutator: mutator
        )
        completion(result.content, result.pending)
    }

    /// The notification once the wait ends, and the new pending-push state (nil leaves it as is).
    /// A locked splice needs nothing from the user, so it shows nothing. A payment left
    /// unacknowledged is flagged for the app, as in the lsp_to_user wait.
    static func content(
        wait: LspSpliceWait,
        receipts: LSPWakeReceipts,
        paymentLeftForForeground: Bool,
        base: UNMutableNotificationContent,
        mutator: NotificationContentMutator
    ) -> (content: UNMutableNotificationContent, pending: Bool?) {
        if receipts.deferStableControlToForeground {
            let content = mutator.buildPending(
                base: base, title: "Payment Pending", body: StabilityNotificationCopy.current.syncPositionInApp
            )
            return (content, true)
        }
        if paymentLeftForForeground {
            let content = mutator.buildPending(
                base: base, title: "Payment Pending", body: "Open app to receive your payment"
            )
            return (content, true)
        }
        if receipts.received {
            let usd = LSPWakeReceipts.usd(receipts.receivedAmountSats, price: receipts.price)
            return (mutator.buildForStabilityReceived(
                base: base, amountSats: receipts.receivedAmountSats, usd: usd, btcPrice: receipts.price
            ), nil)
        }
        if wait.outcome == .locked {
            return (mutator.buildEmpty(base: base), nil)
        }
        let content = mutator.buildPending(base: base, title: LspSpliceNotice.title, body: LspSpliceNotice.body)
        return (content, nil)
    }
}
