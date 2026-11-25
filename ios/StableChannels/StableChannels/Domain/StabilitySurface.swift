import Foundation

/// The two lists on the History screen.
enum HistorySegment: Hashable {
    case orders, payments
}

struct StabilitySurface: Equatable {
    let showsBalanceBar: Bool
    let showsTradeButtons: Bool
    let showsStablePosition: Bool
    let showsOrders: Bool
    let initialHistorySegment: HistorySegment
    let notificationBanner: String
    let notificationsRequired: String
    let pushTitle: String
    let pushDetail: String

    init(stabilityEnabled: Bool) {
        showsBalanceBar = stabilityEnabled
        showsTradeButtons = stabilityEnabled
        showsStablePosition = stabilityEnabled
        showsOrders = stabilityEnabled
        initialHistorySegment = stabilityEnabled ? .orders : .payments
        if stabilityEnabled {
            notificationBanner = String(
                localized: "notifications_disabled_subtitle",
                defaultValue: "Enable notifications for stability payments"
            )
            notificationsRequired = String(
                localized: "info_notifications_needed",
                defaultValue: "Notifications are required to receive stability payments while the app is closed."
            )
            pushTitle = String(
                localized: "info_push_description",
                defaultValue: "Background payments and stability updates"
            )
            pushDetail = String(
                localized: "info_push_connectivity",
                defaultValue: "Your device maintains a persistent connection to receive incoming payments and stability updates."
            )
        } else {
            notificationBanner = String(
                localized: "notifications_disabled_subtitle_no_stability",
                defaultValue: "Enable notifications to receive payments while the app is closed"
            )
            notificationsRequired = String(
                localized: "info_notifications_needed_no_stability",
                defaultValue: "Notifications are required to receive payments while the app is closed."
            )
            pushTitle = String(
                localized: "info_push_description_no_stability",
                defaultValue: "Background payments"
            )
            pushDetail = String(
                localized: "info_push_connectivity_no_stability",
                defaultValue: "Your device maintains a persistent connection to receive incoming payments."
            )
        }
    }

    static let current = StabilitySurface(stabilityEnabled: BuildFeatures.stability)
}
