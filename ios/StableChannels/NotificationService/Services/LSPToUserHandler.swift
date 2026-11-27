import Foundation
import UserNotifications
import LDKNode

/// Handler for "lsp_to_user" direction - wait for incoming payment
final class LSPToUserHandler: PaymentHandler {
    var direction: PaymentDirection { .lspToUser }

    func handle(
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher,
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool?) -> Void
    ) {
        let startTime = Date()
        let lspSplice = SpliceOwnership.expectsLspSplice(UserDefaults(suiteName: Constants.appGroup))
        let timeout: TimeInterval = 22
        var receipts = LSPWakeReceipts()

        eventLoop: while Date().timeIntervalSince(startTime) < timeout {
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
                case .next:
                    break
                case .controlHandled, .stop:
                    break eventLoop
                }
            case .spliceNegotiated, .channelReady, .spliceNegotiationFailed:
                // Commit splice details before acking; a failure while a splice is pending is
                // left for the foreground.
                guard SpliceEventAcknowledgement.backgroundMayAcknowledge(
                    event, db: db, node: node, lspSplice: lspSplice
                )
                else { break eventLoop }
                try? node.eventHandled()
            case .paymentSuccessful, .paymentFailed:
                // Not acked on a failed write: the foreground replays and records it.
                guard recordOutgoingOutcome(event, node: node, db: db) else { break eventLoop }
                try? node.eventHandled()
            default:
                try? node.eventHandled()
            }
        }

        self.finishHandling(
            received: receipts.received,
            handledStableControl: receipts.handledStableControl,
            deferStableControlToForeground: receipts.deferStableControlToForeground,
            price: receipts.price,
            receivedAmountSats: receipts.receivedAmountSats,
            baseContent: baseContent,
            mutator: mutator,
            completion: completion
        )
    }

    private func finishHandling(
        received: Bool,
        handledStableControl: Bool,
        deferStableControlToForeground: Bool,
        price: Double,
        receivedAmountSats: UInt64,
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool?) -> Void
    ) {
        var content: UNMutableNotificationContent
        var shouldPersist: Bool? = nil

        if deferStableControlToForeground {
            content = mutator.buildPending(
                base: baseContent,
                title: "Payment Pending",
                body: StabilityNotificationCopy.current.syncPositionInApp
            )
            shouldPersist = true
        } else if handledStableControl {
            content = mutator.buildEmpty(base: baseContent)
            shouldPersist = false
        } else if received {
            let usd = LSPWakeReceipts.usd(receivedAmountSats, price: price)
            content = mutator.buildForStabilityReceived(
                base: baseContent,
                amountSats: receivedAmountSats,
                usd: usd,
                btcPrice: price
            )
            shouldPersist = false
        } else {
            content = mutator.buildPending(
                base: baseContent,
                title: "Payment Pending",
                body: "Open app to receive your payment"
            )
            shouldPersist = true
        }

        completion(content, shouldPersist)
    }
}

/// The incoming payments of a wake the LSP started: a stable-control message, a signed stability
/// settlement, or an ordinary receipt. Shared by the lsp_to_user and lsp_splice_in waits.
struct LSPWakeReceipts {
    enum Step {
        /// Acknowledged, or ignored; the wait goes on.
        case next
        /// A stable-control message was handled and acknowledged.
        case controlHandled
        /// The event is left unacknowledged and the wait must end.
        case stop
    }

    private(set) var price = 0.0
    private(set) var received = false
    private(set) var receivedAmountSats: UInt64 = 0
    private(set) var handledStableControl = false
    private(set) var deferStableControlToForeground = false

    static func usd(_ sats: UInt64, price: Double) -> Double {
        return BitcoinUnits.usd(sats: sats, price: price)
    }

    mutating func receive(
        paymentId: PaymentId?,
        paymentHash: PaymentHash,
        amountMsat: UInt64,
        customRecords: [CustomTlvRecord],
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher
    ) -> Step {
        let payId = paymentId.map { "\($0)" } ?? "\(paymentHash)"
        if price <= 0 {
            price = priceFetcher.fetchPrice()
        }

        let stableControl = StableControlParser.handleStableControl(
            node: node,
            db: db,
            priceFetcher: priceFetcher,
            customRecords: customRecords,
            amountMsat: amountMsat
        )
        switch stableControl {
        case .handled:
            try? node.eventHandled()
            handledStableControl = true
            deferStableControlToForeground = false
            return .controlHandled
        case .deferToForeground:
            deferStableControlToForeground = true
            handledStableControl = false
            return .stop
        case .none:
            break
        }

        guard amountMsat >= 1000 else {
            try? node.eventHandled()
            return .next
        }

        // Only a valid signed settlement record makes this a stability payment
        // (issue #270): invalid or replayed records are acked and ignored, and
        // payments without one are ordinary Lightning receipts.
        var settlementId: String?
        let isStability: Bool
        switch StableControlParser.signedSettlementStatus(
            node: node,
            db: db,
            customRecords: customRecords,
            amountMsat: amountMsat
        ) {
        case .valid(let id):
            isStability = true
            settlementId = id
        case .invalid(let reason):
            // The sats arrived regardless, so record a Lightning receipt, not backing.
            NSLog("[NSE] invalid signed stability settlement, recording as lightning: \(reason)")
            isStability = false
        case .duplicate(let id):
            NSLog("[NSE] ignoring replayed signed stability settlement: \(id)")
            try? node.eventHandled()
            return .next
        case .stateUnavailable:
            // Do not ack: the sats arrived but local channel state is unreadable, so
            // recording now would dedupe the payment id and lose the backing credit.
            NSLog("[NSE] channel state unavailable for signed settlement, deferring to foreground")
            return .stop
        case .none:
            isStability = false
        }
        if isStability {
            let amountSats = amountMsat / 1000
            switch db.recordPayment(
                paymentId: payId,
                paymentType: "stability",
                direction: "received",
                amountMsat: amountMsat,
                amountUSD: Self.usd(amountSats, price: price),
                btcPrice: price,
                backingDeltaSats: Int64(amountSats),
                userChannelId: db.activeUserChannelId(),
                settlementId: settlementId
            ) {
            case .inserted, .duplicate:
                try? node.eventHandled()
                received = true
                receivedAmountSats = amountSats
                deferStableControlToForeground = false
                return .next
            case .failed, .missingChannelRow:
                return .stop
            }
        }
        // Regular payment - just record it
        let result = db.recordPayment(
            paymentId: payId,
            paymentType: "lightning",
            direction: "received",
            amountMsat: amountMsat,
            amountUSD: Self.usd(amountMsat / 1000, price: price),
            btcPrice: price,
            backingDeltaSats: nil,
            userChannelId: nil,
            settlementId: nil
        )
        switch result {
        case .inserted, .duplicate:
            try? node.eventHandled()
            return .next
        case .failed, .missingChannelRow:
            return .stop
        }
    }
}
