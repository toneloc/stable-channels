import Foundation

struct StabilitySurface: Equatable {
    let showsBalanceBar: Bool
    let showsTradeButtons: Bool
    let showsStablePosition: Bool

    init(stabilityEnabled: Bool) {
        showsBalanceBar = stabilityEnabled
        showsTradeButtons = stabilityEnabled
        showsStablePosition = stabilityEnabled
    }

    static let current = StabilitySurface(stabilityEnabled: BuildFeatures.stability)
}
