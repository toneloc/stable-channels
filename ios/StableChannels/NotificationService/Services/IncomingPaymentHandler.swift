import Foundation
import UserNotifications
import LDKNode

/// Handler for "incoming_payment" direction - wake node to receive pending payments
final class IncomingPaymentHandler: PaymentHandler {
    var direction: PaymentDirection { .incomingPayment }

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
        var persistenceFailed = false
        var handledStableControl = false
        var deferStableControlToForeground = false
        var totalMsat: UInt64 = 0
        var price = 0.0

        eventLoop: while Date().timeIntervalSince(startTime) < timeout {
            guard let event = node.nextEvent() else {
                Thread.sleep(forTimeInterval: 0.5)
                continue
            }
            switch event {
            case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
                if price <= 0 { price = priceFetcher.fetchPrice() }
                let payId = paymentId.map { "\($0)" } ?? "\(paymentHash)"

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
                case .none: break
                }

                guard amountMsat >= 1000 else {
                    try? node.eventHandled()
                    break
                }

                if StableControlParser.isStabilityPayment(customRecords) {
                    let result = db.recordPayment(
                        paymentId: payId,
                        paymentType: "stability",
                        direction: "received",
                        amountMsat: amountMsat,
                        amountUSD: self.calculateUSD(amountMsat / 1000, price: price),
                        btcPrice: price,
                        backingDeltaSats: Int64(amountMsat / 1000),
                        userChannelId: db.activeUserChannelId()
                    )
                    switch result {
                    case .inserted, .duplicate:
                        try? node.eventHandled()
                        received = true
                        totalMsat += amountMsat
                    case .failed, .missingChannelRow:
                        persistenceFailed = true
                    }
                } else {
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
                        totalMsat += amountMsat
                        received = true
                    case .failed, .missingChannelRow:
                        persistenceFailed = true
                    }
                }
            default:
                try? node.eventHandled()
            }
            if persistenceFailed { break eventLoop }
        }

        self.finishHandling(
            received: received,
            handledStableControl: handledStableControl,
            deferStableControlToForeground: deferStableControlToForeground,
            persistenceFailed: persistenceFailed,
            totalMsat: totalMsat,
            price: price,
            baseContent: baseContent,
            mutator: mutator,
            completion: completion
        )
    }

    private func finishHandling(
        received: Bool,
        handledStableControl: Bool,
        deferStableControlToForeground: Bool,
        persistenceFailed: Bool,
        totalMsat: UInt64,
        price: Double,
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
        } else if !received && persistenceFailed {
            content = mutator.buildPending(
                base: baseContent,
                title: "Payment Pending",
                body: "Open app to finish recording your payment"
            )
            shouldPersist = true
        } else if !received {
            content = mutator.buildEmpty(base: baseContent)
            shouldPersist = nil
        } else {
            let totalSats = totalMsat / 1000
            let usd = Double(totalSats) / 100_000_000.0 * price
            content = mutator.buildForReceived(base: baseContent, amountSats: totalSats, usd: usd, btcPrice: price)
            shouldPersist = false
        }

        completion(content, shouldPersist)
    }

    private func calculateUSD(_ sats: UInt64, price: Double) -> Double {
        Double(sats) / 100_000_000.0 * price
    }
}
