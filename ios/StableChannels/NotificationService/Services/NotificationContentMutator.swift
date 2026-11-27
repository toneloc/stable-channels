import Foundation
import UserNotifications

/// Mutates notification content for different payment states
protocol NotificationContentMutator {
    func buildForReceived(base: UNMutableNotificationContent, amountSats: UInt64, usd: Double, btcPrice: Double)
        -> UNMutableNotificationContent
    func buildForStabilityReceived(base: UNMutableNotificationContent, amountSats: UInt64, usd: Double,
                                   btcPrice: Double) -> UNMutableNotificationContent
    func buildForSent(base: UNMutableNotificationContent, amountSats: UInt64, dollars: Double)
        -> UNMutableNotificationContent
    func buildPending(base: UNMutableNotificationContent, title: String, body: String) -> UNMutableNotificationContent
    func buildEmpty(base: UNMutableNotificationContent) -> UNMutableNotificationContent
    func buildStablePosition(base: UNMutableNotificationContent) -> UNMutableNotificationContent
}

final class DefaultNotificationContentMutator: NotificationContentMutator {
    private let copy: StabilityNotificationCopy

    init(copy: StabilityNotificationCopy = .current) {
        self.copy = copy
    }

    func buildForReceived(base: UNMutableNotificationContent, amountSats: UInt64, usd: Double,
                          btcPrice: Double) -> UNMutableNotificationContent {
        base.title = "Payment Received"
        base.body = btcPrice > 0 ? String(format: "$%.2f received", usd) : "\(amountSats) sats received"
        base.sound = .default
        return base
    }

    func buildForStabilityReceived(base: UNMutableNotificationContent, amountSats: UInt64, usd: Double,
                                   btcPrice: Double) -> UNMutableNotificationContent {
        base.title = copy.receivedTitle
        base.body = btcPrice > 0 ? String(format: "$%.2f received", usd) : "\(amountSats) sats received"
        base.sound = .default
        return base
    }

    func buildForSent(base: UNMutableNotificationContent, amountSats: UInt64,
                      dollars: Double) -> UNMutableNotificationContent {
        base.title = copy.sentTitle
        base.body = copy.sentBody(sats: amountSats, dollars: dollars)
        base.sound = .default
        return base
    }

    func buildPending(base: UNMutableNotificationContent, title: String, body: String) -> UNMutableNotificationContent {
        base.title = title
        base.body = body
        return base
    }

    func buildEmpty(base: UNMutableNotificationContent) -> UNMutableNotificationContent {
        base.title = ""
        base.body = ""
        base.sound = nil
        return base
    }

    func buildStablePosition(base: UNMutableNotificationContent) -> UNMutableNotificationContent {
        base.title = copy.checkTitle
        base.body = copy.checkBody
        return base
    }
}
