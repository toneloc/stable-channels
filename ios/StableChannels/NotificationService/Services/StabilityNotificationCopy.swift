import Foundation

/// Notification text for stability settlements, kept in one place. The App Store build has no stability, so a
/// wallet restored with a stable position from an earlier build sees plain payment wording instead.
struct StabilityNotificationCopy: Equatable {
    let receivedTitle: String
    let sentTitle: String
    let checkTitle: String
    let checkBody: String
    let processInApp: String
    let finishSyncInApp: String
    let syncPositionInApp: String
    private let sentBodySuffix: String

    init(stabilityEnabled: Bool) {
        if stabilityEnabled {
            receivedTitle = "Stability Payment Received"
            sentTitle = "Stability Payment Sent"
            sentBodySuffix = " to maintain stable position"
            checkTitle = "Stability Check"
            checkBody = "Position is stable"
            processInApp = "Open app to process stability payment"
            finishSyncInApp = "Open app to finish syncing the stability payment"
            syncPositionInApp = "Open app to sync stable position"
        } else {
            receivedTitle = "Payment Received"
            sentTitle = "Payment Sent"
            sentBodySuffix = ""
            checkTitle = "Balance Check"
            checkBody = "Balance is up to date"
            processInApp = "Open app to process a payment"
            finishSyncInApp = "Open app to finish syncing a payment"
            syncPositionInApp = "Open app to sync your balance"
        }
    }

    func sentBody(sats: UInt64, dollars: Double) -> String {
        String(format: "Sent %d sats ($%.2f)", sats, dollars) + sentBodySuffix
    }

    static let current = StabilityNotificationCopy(stabilityEnabled: BuildFeatures.stability)
}
