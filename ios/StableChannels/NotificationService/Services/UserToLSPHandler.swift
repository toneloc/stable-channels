import Foundation
import UserNotifications
import LDKNode

/// Handler for "user_to_lsp" direction - calculate and send payment to LSP
final class UserToLSPHandler: PaymentHandler {
    var direction: PaymentDirection { .userToLsp }

    func handle(
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher,
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool) -> Void
    ) {
        // Check pending outgoing
        guard db.reconcilePendingOutgoingPayment(node: node) else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Sent",
                    body: "Open app to finish syncing the stability payment"
                ),
                true
            )
            return
        }

        // Cooldown check
        let shared = UserDefaults(suiteName: Constants.appGroup)
        shared?.synchronize()
        let lastSent = shared?.double(forKey: "nse_last_stability_sent") ?? 0
        if lastSent > 0 && Date().timeIntervalSince1970 - lastSent < 120 {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), false)
            return
        }

        // Read channel state
        guard let channelState = db.readChannelState() else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
            return
        }

        let backingSats = channelState.backingSats
        guard channelState.expectedUSD >= 0.01 else {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), false)
            return
        }

        // Fetch price
        let price = priceFetcher.fetchPrice()
        guard price > 0 else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
            return
        }

        // Calculate stability payment
        let stableUSDValue = Double(backingSats) / Constants.satsInBTC * price
        let targetUSD = channelState.expectedUSD
        let dollarsFromPar = stableUSDValue - targetUSD
        let percentFromPar = targetUSD > 0 ? abs(dollarsFromPar / targetUSD) * 100.0 : 0.0

        // Within threshold - no payment needed
        guard percentFromPar >= Constants.stabilityThresholdPercent && abs(dollarsFromPar) >= 0.25 else {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), false)
            return
        }

        // User is above expected (price rose) - should pay LSP
        guard stableUSDValue > targetUSD else {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), false)
            return
        }

        // Calculate amount
        let dollarsAbs = abs(dollarsFromPar)
        let btcAmount = dollarsAbs / price
        let amountMsat = UInt64(btcAmount * Constants.satsInBTC * 1000)
        let amountSats = amountMsat / 1000

        // Claim slot
        guard db.claimPendingSend(amountMsat: amountMsat, price: price) else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
            return
        }

        // Send keysend
        do {
            let tlvRecord = CustomTlvRecord(typeNum: Constants.stableChannelTLVType, value: Data([1]))
            let paymentId = try node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat: amountMsat,
                nodeId: Constants.lspPubkey,
                routeParameters: nil,
                customTlvs: [tlvRecord]
            )

            // Payment ID Guard
            let guardSaved = db.setPendingSendPaymentId(paymentId: "\(paymentId)")

            // Update cooldown
            shared?.set(Date().timeIntervalSince1970, forKey: "nse_last_stability_sent")
            shared?.synchronize()

            guard guardSaved else {
                completion(
                    mutator.buildPending(
                        base: baseContent,
                        title: "Payment Sent",
                        body: "Open app to finish syncing the stability payment"
                    ),
                    true
                )
                return
            }

            // Record payment
            let result = db.recordPayment(
                paymentId: "\(paymentId)",
                paymentType: "stability",
                direction: "sent",
                amountMsat: amountMsat,
                amountUSD: dollarsAbs,
                btcPrice: price,
                backingDeltaSats: -Int64(amountSats),
                userChannelId: channelState.userChannelId
            )

            switch result {
            case .inserted, .duplicate:
                db.clearPendingSend()
                completion(mutator.buildForSent(base: baseContent, amountSats: amountSats, dollars: dollarsAbs), false)
            case .failed, .missingChannelRow:
                completion(
                    mutator
                        .buildPending(
                            base: baseContent,
                            title: "Payment Sent",
                            body: "Open app to finish syncing the stability payment"
                        ),
                    true
                )
            }
        } catch {
            db.clearPendingSend()
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
        }
    }
}
