import Foundation

enum TradeDirection {
    case buy // drag left: grow BTC
    case sell // drag right: grow USD
}

struct TradeRequest: Identifiable {
    let id = UUID()
    let direction: TradeDirection
    let amountUSD: Double
}
