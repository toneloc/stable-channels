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
        let timeout: TimeInterval = 22
        var received = false
        var handledStableControl = false
        var deferStableControlToForeground = false
        var price = 0.0
        var receivedAmountSats: UInt64 = 0

        eventLoop: while Date().timeIntervalSince(startTime) < timeout {
            guard let event = node.nextEvent() else {
                Thread.sleep(forTimeInterval: 0.5)
                continue
            }
            switch event {
            case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
                let payId = paymentId.map { "\($0)" } ?? "\(paymentHash)"
                if price <= 0 {
                    price = priceFetcher.fetchPrice()
                }

                let stableControl = StableControlParser.handleStableControl(
                    node: node,
                    db: db,
                    priceFetcher: priceFetcher,
                    customRecords: customRecords
                )
                switch stableControl {
                case .handled:
                    try? node.eventHandled()
                    handledStableControl = true
                    deferStableControlToForeground = false
                    break eventLoop
                case .deferToForeground:
                    deferStableControlToForeground = true
                    handledStableControl = false
                    break eventLoop
                case .none:
                    break
                }

                guard amountMsat >= 1000 else {
                    try? node.eventHandled()
                    break
                }

                let isStability = StableControlParser.isStabilityPayment(customRecords)
                if isStability {
                    let amountSats = amountMsat / 1000
                    switch db.recordPayment(
                        paymentId: payId,
                        paymentType: "stability",
                        direction: "received",
                        amountMsat: amountMsat,
                        amountUSD: self.calculateUSD(amountSats, price: price),
                        btcPrice: price,
                        backingDeltaSats: Int64(amountSats),
                        userChannelId: db.activeUserChannelId()
                    ) {
                    case .inserted, .duplicate:
                        try? node.eventHandled()
                        received = true
                        receivedAmountSats = amountSats
                        deferStableControlToForeground = false
                    case .failed, .missingChannelRow:
                        break eventLoop
                    }
                } else {
                    // Regular payment - just record it
                    let result = db.recordPayment(
                        paymentId: payId,
                        paymentType: "lightning",
                        direction: "received",
                        amountMsat: amountMsat,
                        amountUSD: self.calculateUSD(amountMsat / 1000, price: price),
                        btcPrice: price,
                        backingDeltaSats: nil,
                        userChannelId: nil
                    )
                    switch result {
                    case .inserted, .duplicate:
                        try? node.eventHandled()
                    case .failed, .missingChannelRow:
                        break eventLoop
                    }
                }
            default:
                try? node.eventHandled()
            }
        }

        self.finishHandling(
            received: received,
            handledStableControl: handledStableControl,
            deferStableControlToForeground: deferStableControlToForeground,
            price: price,
            receivedAmountSats: receivedAmountSats,
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
                body: "Open app to sync stable position"
            )
            shouldPersist = true
        } else if handledStableControl {
            content = mutator.buildEmpty(base: baseContent)
            shouldPersist = false
        } else if received {
            let usd = calculateUSD(receivedAmountSats, price: price)
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

    private func calculateUSD(_ sats: UInt64, price: Double) -> Double {
        return Double(sats) / 100_000_000.0 * price
    }
}
