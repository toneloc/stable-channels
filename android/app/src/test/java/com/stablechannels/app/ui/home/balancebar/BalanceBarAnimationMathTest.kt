package com.stablechannels.app.ui.home.balancebar

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class BalanceBarAnimationMathTest {

    @Test
    fun thumbScaleProgressBoundaryAndPeakValues() {
        // Resting progress
        assertEquals(1.0f, BalanceBarAnimationMath.thumbScale(0.0f), 0.001f)
        assertEquals(1.0f, BalanceBarAnimationMath.thumbScale(-0.1f), 0.001f)
        assertEquals(1.0f, BalanceBarAnimationMath.thumbScale(0.6f), 0.001f)
        assertEquals(1.0f, BalanceBarAnimationMath.thumbScale(1.0f), 0.001f)

        // Peak at 0.22 progress
        assertEquals(1.35f, BalanceBarAnimationMath.thumbScale(0.22f), 0.001f)

        // Mid-way surge (0.11 progress -> 1.0 + 0.5 * 0.35 = 1.175)
        assertEquals(1.175f, BalanceBarAnimationMath.thumbScale(0.11f), 0.001f)

        // Mid-way recovery (0.41 progress -> 1.35 - 0.5 * 0.35 = 1.175)
        assertEquals(1.175f, BalanceBarAnimationMath.thumbScale(0.41f), 0.001f)
    }

    @Test
    fun floodScaleProgressBoundaryAndLinearInterpolation() {
        assertEquals(0.01f, BalanceBarAnimationMath.floodScale(0.0f), 0.001f)
        assertEquals(0.01f, BalanceBarAnimationMath.floodScale(0.05f), 0.001f)
        assertEquals(1.0f, BalanceBarAnimationMath.floodScale(0.55f), 0.001f)
        assertEquals(1.0f, BalanceBarAnimationMath.floodScale(1.0f), 0.001f)

        // Mid-point expansion (0.30 progress -> 0.01 + 0.5 * 0.99 = 0.505)
        assertEquals(0.505f, BalanceBarAnimationMath.floodScale(0.30f), 0.001f)
    }

    @Test
    fun floodAlphaProgressRampUpAndFadeOut() {
        assertEquals(0.0f, BalanceBarAnimationMath.floodAlpha(0.0f), 0.001f)
        assertEquals(0.0f, BalanceBarAnimationMath.floodAlpha(0.05f), 0.001f)
        assertEquals(0.0f, BalanceBarAnimationMath.floodAlpha(0.65f), 0.001f)
        assertEquals(0.0f, BalanceBarAnimationMath.floodAlpha(1.0f), 0.001f)

        // Peak alpha at 0.28 progress
        assertEquals(0.55f, BalanceBarAnimationMath.floodAlpha(0.28f), 0.001f)

        // Halfway ramp up (0.165 progress -> 0.5 * 0.55 = 0.275)
        assertEquals(0.275f, BalanceBarAnimationMath.floodAlpha(0.165f), 0.001f)

        // Halfway fade out (0.465 progress -> 0.55 * 0.5 = 0.275)
        assertEquals(0.275f, BalanceBarAnimationMath.floodAlpha(0.465f), 0.001f)
    }

    @Test
    fun settleFractionInterpolationPhases() {
        // Zero progress returns null
        assertNull(BalanceBarAnimationMath.settleFraction(0.5f, 0.8f, 0.0f))

        // Pre-settle holding phase (< 0.45) holds initial fraction
        assertEquals(0.5f, BalanceBarAnimationMath.settleFraction(0.5f, 0.8f, 0.2f) ?: 0f, 0.001f)
        assertEquals(0.5f, BalanceBarAnimationMath.settleFraction(0.5f, 0.8f, 0.44f) ?: 0f, 0.001f)

        // Mid-point settle (0.725 progress -> phase 0.5 -> 0.5 + (0.8 - 0.5) * 0.5 = 0.65)
        val midSettle = BalanceBarAnimationMath.settleFraction(0.5f, 0.8f, 0.725f)
        assertNotNull(midSettle)
        assertEquals(0.65f, midSettle ?: 0f, 0.001f)

        // Full completion (1.0 progress -> 0.8)
        val finalSettle = BalanceBarAnimationMath.settleFraction(0.5f, 0.8f, 1.0f)
        assertNotNull(finalSettle)
        assertEquals(0.8f, finalSettle ?: 0f, 0.001f)
    }
}
