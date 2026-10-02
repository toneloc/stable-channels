package com.stablechannels.app.ui.home.balancebar

import kotlin.math.abs
import kotlin.math.hypot

/**
 * Pure interaction model for the balance bar slider. Defines mathematical coordinate translation
 * and touch hit-testing without UI side effects.
 */
object BalanceBarInteraction {
    const val DEFAULT_TAP_THRESHOLD: Float = 5.0f
    const val DEFAULT_THUMB_HIT_MULTIPLIER: Float = 1.5f

    /**
     * Mathematical formula unifying coordinate translation across platforms: fraction =
     * clamp(initialFraction + translationX / barWidth, 0.0, 1.0)
     */
    fun calculateTargetFraction(
        initialFraction: Float,
        translationX: Float,
        barWidth: Float,
    ): Float {
        if (barWidth <= 0f) return initialFraction
        val proposed = initialFraction + (translationX / barWidth)
        return proposed.coerceIn(0f, 1f)
    }

    /** Computes horizontal thumb center position along the track within bounds. */
    fun calculateThumbPosition(
        fraction: Float,
        barWidth: Float,
        thumbDiameter: Float,
    ): Float {
        if (barWidth <= 0f) return thumbDiameter / 2f
        val clamped = fraction.coerceIn(0f, 1f)
        return (thumbDiameter / 2f) + (barWidth - thumbDiameter) * clamped
    }

    /** Evaluates if gesture displacement qualifies as a tap rather than a drag. */
    fun isTap(
        translationX: Float,
        translationY: Float = 0.0f,
        threshold: Float = DEFAULT_TAP_THRESHOLD,
    ): Boolean {
        return hypot(translationX.toDouble(), translationY.toDouble()).toFloat() < threshold
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
}
