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
    func buildStablePosition(base: UNMutableNotificationContent, body: String) -> UNMutableNotificationContent
}

final class DefaultNotificationContentMutator: NotificationContentMutator {
    func buildForReceived(base: UNMutableNotificationContent, amountSats: UInt64, usd: Double,
                          btcPrice: Double) -> UNMutableNotificationContent {
        base.title = "Payment Received"
        base.body = btcPrice > 0 ? String(format: "$%.2f received", usd) : "\(amountSats) sats received"
        base.sound = .default
        return base
    }

    func buildForStabilityReceived(base: UNMutableNotificationContent, amountSats: UInt64, usd: Double,
                                   btcPrice: Double) -> UNMutableNotificationContent {
        base.title = "Stability Payment Received"
        base.body = btcPrice > 0 ? String(format: "$%.2f received", usd) : "\(amountSats) sats received"
        base.sound = .default
        return base
    }

    func buildForSent(base: UNMutableNotificationContent, amountSats: UInt64,
                      dollars: Double) -> UNMutableNotificationContent {
        base.title = "Stability Payment Sent"
        base.body = String(format: "Sent %d sats ($%.2f) to maintain stable position", amountSats, dollars)
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

    func buildStablePosition(base: UNMutableNotificationContent, body: String) -> UNMutableNotificationContent {
        base.title = "Stability Check"
        base.body = body
        return base
    }
}
