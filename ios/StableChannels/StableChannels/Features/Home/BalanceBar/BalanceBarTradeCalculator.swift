import Foundation

/// Pure domain service calculating trade requests, fraction clamping, and interaction geometry.
/// Zero UI framework dependencies (Functional Core).
enum BalanceBarTradeCalculator {
    static let defaultThumbDiameter: CGFloat = 22.0
    static let defaultMinTradeUSD: Double = 1.0
    static let defaultTapThreshold: CGFloat = 5.0
    static let defaultThumbHitMultiplier: CGFloat = 1.5

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

    /// Mathematical formula unifying coordinate translation across platforms with thumb inset:
    /// fraction = clamp(initialFraction + translation / usableWidth, 0.0, 1.0)
    static func calculateTargetFraction(
        initialFraction: CGFloat,
        translationX: CGFloat,
        barWidth: CGFloat,
        thumbDiameter: CGFloat = defaultThumbDiameter
    ) -> CGFloat {
        let usableWidth = barWidth - thumbDiameter
        guard usableWidth > 0 else { return initialFraction }
        let proposed = initialFraction + (translationX / usableWidth)
        return min(max(proposed, 0.0), 1.0)
    }

    /// Computes horizontal thumb center position along the track with thumb radius inset.
    /// Ensures thumb remains flush within bounds at 0% and 100%, and guarantees remaining
    /// BTC reserve is visually displayed on the track when clamped at the sell limit.
    static func calculateThumbPosition(
        fraction: CGFloat,
        barWidth: CGFloat,
        thumbDiameter: CGFloat = defaultThumbDiameter
    ) -> CGFloat {
        let usableWidth = barWidth - thumbDiameter
        guard usableWidth > 0 else { return barWidth / 2 }
        let radius = thumbDiameter / 2
        let clamped = min(max(fraction, 0.0), 1.0)
        return radius + (clamped * usableWidth)
    }

    /// Evaluates if gesture displacement qualifies as a tap rather than a drag based on cumulative distance.
    static func isTap(
        totalDistance: CGFloat,
        threshold: CGFloat = defaultTapThreshold
    ) -> Bool {
        totalDistance <= threshold
    }

    /// Evaluates if gesture displacement qualifies as a tap rather than a drag.
    static func isTap(
        translationX: CGFloat,
        translationY: CGFloat = 0.0,
        threshold: CGFloat = defaultTapThreshold
    ) -> Bool {
        hypot(translationX, translationY) <= threshold
    }

    /// Determines if an initial touch falls within the interactive hit area of the thumb.
    static func isWithinThumb(
        touchX: CGFloat,
        thumbX: CGFloat,
        thumbDiameter: CGFloat,
        multiplier: CGFloat = defaultThumbHitMultiplier
    ) -> Bool {
        abs(touchX - thumbX) < thumbDiameter * multiplier
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
    /// 100% pure function: identical inputs always yield identical outputs.
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
                isValidTrade: false
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
                isValidTrade: false
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

        return BalanceBarTradeEvaluation(
            direction: direction,
            requestedUSD: requestedUSD,
            clampedUSD: clampedUSD,
            isValidTrade: isValid
        )
    }
}
