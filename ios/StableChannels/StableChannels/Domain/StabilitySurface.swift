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
    let pushTitle: String
    let pushDetail: String

    init(stabilityEnabled: Bool) {
        showsBalanceBar = stabilityEnabled
        showsTradeButtons = stabilityEnabled
        showsStablePosition = stabilityEnabled
        showsOrders = stabilityEnabled
        initialHistorySegment = stabilityEnabled ? .orders : .payments
        if stabilityEnabled {
            pushTitle = String(
                localized: "info_push_description",
                defaultValue: "Background payments and stability updates"
            )
            pushDetail = String(
                localized: "info_push_connectivity",
                defaultValue: "Your device maintains a persistent connection to receive incoming payments and stability updates."
            )
        } else {
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

    /// A trade button's action, or nil so the button is not shown when the build has no stability.
    func tradeAction(_ action: @escaping () -> Void) -> (() -> Void)? {
        showsTradeButtons ? action : nil
    }
}
