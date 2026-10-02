import Foundation

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
    let tradeRequest: TradeRequest?
}

struct ClampedFractionResult: Equatable, Sendable {
    let fraction: CGFloat
    let isAtSellLimit: Bool
}
