import Foundation
import UIKit

enum TradeDirection: Equatable, Sendable {
    case buy // drag left: grow BTC
    case sell // drag right: grow USD
}

struct TradeRequest: Identifiable, Equatable, Sendable {
    let id: UUID
    let direction: TradeDirection
    let amountUSD: Double

    init(id: UUID = UUID(), direction: TradeDirection, amountUSD: Double) {
        self.id = id
        self.direction = direction
        self.amountUSD = amountUSD
    }
}

struct BalanceBarTradeEvaluation: Equatable, Sendable {
    let direction: TradeDirection?
    let requestedUSD: Double
    let clampedUSD: Double
    let isValidTrade: Bool
}

struct ClampedFractionResult: Equatable, Sendable {
    let fraction: CGFloat
    let isAtSellLimit: Bool
}

protocol BalanceBarHaptics: AnyObject {
    func tick()
    func impact()
    func warning()
}

final class SystemBalanceBarHaptics: BalanceBarHaptics {
    func tick() {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
    }

    func impact() {
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
    }

    func warning() {
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
    }
}
