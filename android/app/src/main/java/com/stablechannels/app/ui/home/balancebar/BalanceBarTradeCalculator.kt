package com.stablechannels.app.ui.home.balancebar

import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.round

/**
 * Pure domain service calculating trade requests, fraction clamping, and interaction geometry. Free
 * of UI and Android framework dependencies (Functional Core).
 */
object BalanceBarTradeCalculator {
    const val DEFAULT_THUMB_DIAMETER: Float = 22.0f
    const val DEFAULT_MIN_TRADE_USD: Double = 1.0
    const val DEFAULT_TAP_THRESHOLD: Float = 5.0f
    const val DEFAULT_THUMB_HIT_MULTIPLIER: Float = 1.5f

    /** Determines trade direction from fraction delta. */
    fun tradeDirection(initialFraction: Float, targetFraction: Float): TradeDirection? {
        return when {
            targetFraction > initialFraction -> TradeDirection.SELL
            targetFraction < initialFraction -> TradeDirection.BUY
            else -> null
        }
    }

    /**
     * Mathematical formula unifying coordinate translation across platforms with thumb inset:
     * fraction = clamp(initialFraction + translationX / usableWidth, 0.0, 1.0)
     */
    fun calculateTargetFraction(
        initialFraction: Float,
        translationX: Float,
        barWidth: Float,
        thumbDiameter: Float = DEFAULT_THUMB_DIAMETER,
    ): Float {
        val usableWidth = barWidth - thumbDiameter
        if (usableWidth <= 0f) return initialFraction
        val proposed = initialFraction + (translationX / usableWidth)
        return proposed.coerceIn(0f, 1f)
    }

    /**
     * Computes horizontal thumb center position along the track with thumb radius inset. Ensures
     * thumb remains flush within bounds at 0% and 100%, and guarantees remaining BTC reserve is
     * visually displayed on the track when clamped at the sell limit.
     */
    fun calculateThumbPosition(
        fraction: Float,
        barWidth: Float,
        thumbDiameter: Float = DEFAULT_THUMB_DIAMETER,
    ): Float {
        val usableWidth = barWidth - thumbDiameter
        if (usableWidth <= 0f) return barWidth / 2f
        val radius = thumbDiameter / 2f
        val clamped = fraction.coerceIn(0f, 1f)
        return radius + (clamped * usableWidth)
    }

    /** Evaluates if gesture displacement qualifies as a tap rather than a drag. */
    fun isTap(
        translationX: Float,
        translationY: Float = 0.0f,
        threshold: Float = DEFAULT_TAP_THRESHOLD,
    ): Boolean {
        return hypot(translationX.toDouble(), translationY.toDouble()).toFloat() <= threshold
    }

    /** Determines if an initial touch falls within the interactive hit area of the thumb. */
    fun isWithinThumb(
        touchX: Float,
        thumbX: Float,
        thumbDiameter: Float,
        multiplier: Float = DEFAULT_THUMB_HIT_MULTIPLIER,
    ): Boolean {
        return abs(touchX - thumbX) < thumbDiameter * multiplier
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

    /**
     * Evaluates financial trade viability from fraction movement. 100% pure function: identical
     * inputs always yield identical outputs.
     */
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

        val requestedUSD = round(totalUSD * fractionMoved.toDouble() * 100.0) / 100.0
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
