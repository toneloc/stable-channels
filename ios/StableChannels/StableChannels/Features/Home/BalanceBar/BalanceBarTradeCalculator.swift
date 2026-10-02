import Foundation

/// Pure domain service calculating trade requests, fraction clamping, and financial limits.
/// Zero UI framework dependencies (Functional Core).
enum BalanceBarTradeCalculator {
    static let defaultMinTradeUSD: Double = 1.0

    /// Determines trade direction from fraction delta.
    static func tradeDirection(
        initialFraction: CGFloat,
        targetFraction: CGFloat
    ) -> TradeDirection? {
        if targetFraction > initialFraction {
            return .sell
        } else if targetFraction < initialFraction {
            return .buy
        }
        return nil
    }

    /// Pure function clamping a proposed fraction within physical [0, 1] and financial liquidity bounds.
    static func clampFraction(
        initialFraction: CGFloat,
        rawFraction: CGFloat,
        totalUSD: Double,
        stableUSD: Double,
        maxSellUSD: Double
    ) -> ClampedFractionResult {
        guard totalUSD > 0 else {
            let clamped = min(max(rawFraction, 0.0), 1.0)
            return ClampedFractionResult(fraction: clamped, isAtSellLimit: false)
        }

        let maxSellFraction = CGFloat(max(0.0, maxSellUSD) / totalUSD)
        let maxBuyFraction = CGFloat(max(0.0, stableUSD) / totalUSD)

        let minAllowedFraction = max(0.0, initialFraction - maxBuyFraction)
        let maxAllowedFraction = min(1.0, initialFraction + maxSellFraction)

        let isAtSellLimit = rawFraction > maxAllowedFraction
        let clamped = min(max(rawFraction, minAllowedFraction), maxAllowedFraction)

        return ClampedFractionResult(fraction: clamped, isAtSellLimit: isAtSellLimit)
    }

    /// Evaluates financial trade viability from fraction movement.
    static func calculateSelection(
        initialFraction: CGFloat,
        targetFraction: CGFloat,
        totalUSD: Double,
        stableUSD: Double,
        maxSellUSD: Double,
        minTradeUSD: Double = defaultMinTradeUSD
    ) -> BalanceBarTradeEvaluation {
        guard totalUSD > 0 else {
            return BalanceBarTradeEvaluation(
                direction: nil,
                requestedUSD: 0.0,
                clampedUSD: 0.0,
                isValidTrade: false,
                tradeRequest: nil
            )
        }

        let deltaFraction = targetFraction - initialFraction
        let fractionMoved = abs(deltaFraction)

        guard fractionMoved > 0.001,
              let direction = tradeDirection(initialFraction: initialFraction, targetFraction: targetFraction)
        else {
            return BalanceBarTradeEvaluation(
                direction: nil,
                requestedUSD: 0.0,
                clampedUSD: 0.0,
                isValidTrade: false,
                tradeRequest: nil
            )
        }

        let requestedUSD = (totalUSD * Double(fractionMoved) * 100.0).rounded() / 100.0
        let clampedUSD: Double
        if direction == .sell {
            clampedUSD = min(requestedUSD, max(0.0, maxSellUSD))
        } else {
            clampedUSD = min(requestedUSD, max(0.0, stableUSD))
        }

        let isValid = clampedUSD >= minTradeUSD
        let request = isValid ? TradeRequest(direction: direction, amountUSD: clampedUSD) : nil

        return BalanceBarTradeEvaluation(
            direction: direction,
            requestedUSD: requestedUSD,
            clampedUSD: clampedUSD,
            isValidTrade: isValid,
            tradeRequest: request
        )
    }
}
