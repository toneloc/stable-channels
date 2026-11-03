import Foundation

/// The numbers a trade sheet shows and checks, from what the user typed, the trade limit and the
/// price. Pure, so the fee and limit math is tested without a view.
struct TradeFormModel: Equatable {
    /// Absorbs binary rounding when dollars become whole cents ($0.29 * 100 is 28.999...).
    static let centRoundingTolerance = 1e-7

    let action: TradeAction
    let amountText: String
    let maxTradeUSD: Double
    let price: Double

    /// The most a trade may convert: the stable balance for a buy, the stabilization limit for a sell.
    static func maxTradeUSD(action: TradeAction, expectedUSD: Double, maxSellCents: UInt64) -> Double {
        switch action {
        case .buyBTC: BuyAmountPolicy.maximumUsd(expectedUSD)
        case .sellBTC: Double(maxSellCents) / 100
        }
    }

    /// Whole cents of [usd], rounding binary noise up rather than losing a cent.
    static func cents(_ usd: Double) -> UInt64 {
        UInt64(usd * 100 + centRoundingTolerance)
    }

    var amountUSD: Double {
        Double(amountText) ?? 0
    }

    var feeUSD: Double {
        amountUSD * Constants.stableChannelTradeFeeRate
    }

    var netAmountUSD: Double {
        amountUSD - feeUSD
    }

    var maxTradeCents: UInt64 {
        Self.cents(maxTradeUSD)
    }

    var acceptsAmount: Bool {
        switch action {
        case .buyBTC: BuyAmountPolicy.accepts(amountUsd: amountUSD, balanceUsd: maxTradeUSD)
        case .sellBTC: amountUSD > 0 && amountUSD <= maxTradeUSD
        }
    }

    var exceedsLimit: Bool {
        amountUSD > maxTradeUSD && amountUSD > 0
    }

    var btcAmount: Double {
        price > 0 ? amountUSD / price : 0
    }

    /// The BTC side after the fee.
    var btcAmountFinal: Double {
        price > 0 ? netAmountUSD / price : 0
    }

    /// The USD side of the trade as shown in the result copy: the amount for a buy, the post-fee
    /// USD for a sell.
    var tradedUSD: Double {
        action == .buyBTC ? amountUSD : netAmountUSD
    }
}
