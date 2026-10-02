package com.stablechannels.app.ui.home.balancebar

import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/**
 * Pure domain service calculating trade requests, fraction clamping, and financial limits. Free of
 * UI and Android framework dependencies (Functional Core).
 */
object BalanceBarTradeCalculator {
    const val DEFAULT_MIN_TRADE_USD: Double = 1.0

    /** Determines trade direction from fraction delta. */
    fun tradeDirection(initialFraction: Float, targetFraction: Float): TradeDirection? {
        return when {
            targetFraction > initialFraction -> TradeDirection.SELL
            targetFraction < initialFraction -> TradeDirection.BUY
            else -> null
        }
    }

    /**
     * Pure function clamping a proposed fraction within physical [0, 1] and financial liquidity
     * bounds.
     */
    fun clampFraction(
        initialFraction: Float,
        rawFraction: Float,
        totalUSD: Double,
        stableUSD: Double,
        maxSellUSD: Double,
    ): ClampedFractionResult {
        if (totalUSD <= 0.0) {
            val clamped = rawFraction.coerceIn(0f, 1f)
            return ClampedFractionResult(fraction = clamped, isAtSellLimit = false)
        }

        val maxSellFraction = (max(0.0, maxSellUSD) / totalUSD).toFloat()
        val maxBuyFraction = (max(0.0, stableUSD) / totalUSD).toFloat()

        val minAllowedFraction = max(0f, initialFraction - maxBuyFraction)
        val maxAllowedFraction = min(1f, initialFraction + maxSellFraction)

        val isAtSellLimit = rawFraction > maxAllowedFraction
        val clamped = rawFraction.coerceIn(minAllowedFraction, maxAllowedFraction)

        return ClampedFractionResult(fraction = clamped, isAtSellLimit = isAtSellLimit)
    }

    /** Evaluates financial trade viability from fraction movement. */
    fun calculateSelection(
        initialFraction: Float,
        targetFraction: Float,
        totalUSD: Double,
        stableUSD: Double,
        maxSellUSD: Double,
        minTradeUSD: Double = DEFAULT_MIN_TRADE_USD,
    ): BalanceBarTradeEvaluation {
        if (totalUSD <= 0.0) {
            return BalanceBarTradeEvaluation(
                direction = null,
                requestedUSD = 0.0,
                clampedUSD = 0.0,
                isValidTrade = false,
            )
        }

        val deltaFraction = targetFraction - initialFraction
        val fractionMoved = abs(deltaFraction)

        val direction = tradeDirection(initialFraction, targetFraction)
        if (fractionMoved <= 0.001f || direction == null) {
            return BalanceBarTradeEvaluation(
                direction = null,
                requestedUSD = 0.0,
                clampedUSD = 0.0,
                isValidTrade = false,
            )
        }

        val requestedUSD = kotlin.math.round(totalUSD * fractionMoved.toDouble() * 100.0) / 100.0
        val clampedUSD =
            if (direction == TradeDirection.SELL) {
                min(requestedUSD, max(0.0, maxSellUSD))
            } else {
                min(requestedUSD, max(0.0, stableUSD))
            }

        val isValid = clampedUSD >= minTradeUSD

        return BalanceBarTradeEvaluation(
            direction = direction,
            requestedUSD = requestedUSD,
            clampedUSD = clampedUSD,
            isValidTrade = isValid,
        )
    }
}
