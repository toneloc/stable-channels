package com.stablechannels.app.ui.components

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

class BalanceScaleKinematicsTest {
    private val kinematics = BalanceScaleKinematics()

    @Test
    fun testInitialStageIsResting() {
        val stage = kinematics.evaluate(
            elapsedSinceStart = 0.10f,
            isSyncComplete = false,
            settleElapsed = null
        )
        assertEquals(BalanceScaleKinematics.Stage.Resting, stage)
    }

    @Test
    fun testShimmerStageProgression() {
        // Shimmer begins at 0.25s
        val startStage = kinematics.evaluate(0.25f, false, null)
        assertTrue(startStage is BalanceScaleKinematics.Stage.Shimmer)
        assertEquals(0.0f, (startStage as BalanceScaleKinematics.Stage.Shimmer).progress, 0.001f)

        // Mid-shimmer at 0.70s (0.45s elapsed in 0.90s duration)
        val midStage = kinematics.evaluate(0.70f, false, null)
        assertTrue(midStage is BalanceScaleKinematics.Stage.Shimmer)
        assertEquals(0.5f, (midStage as BalanceScaleKinematics.Stage.Shimmer).progress, 0.01f)
    }

    @Test
    fun testOscillationStage() {
        // Oscillation begins at 1.15s (0.25s + 0.90s)
        val stage = kinematics.evaluate(1.15f, false, null)
        assertTrue(stage is BalanceScaleKinematics.Stage.Oscillating)
        assertEquals(0.0f, (stage as BalanceScaleKinematics.Stage.Oscillating).angle, 0.01f)

        // Peak quarter cycle (1.15s + 0.50s = 1.65s)
        val peakStage = kinematics.evaluate(1.65f, false, null)
        assertTrue(peakStage is BalanceScaleKinematics.Stage.Oscillating)
        assertEquals(4.8f, (peakStage as BalanceScaleKinematics.Stage.Oscillating).angle, 0.1f)
    }

    @Test
    fun testSettlingStageAndBalanced() {
        val settlingStage = kinematics.evaluate(2.0f, true, 0.6f)
        assertTrue(settlingStage is BalanceScaleKinematics.Stage.Settling)
        val settlingAngle = (settlingStage as BalanceScaleKinematics.Stage.Settling).angle
        assertTrue(abs(settlingAngle) < 4.8f)

        val balancedStage = kinematics.evaluate(2.0f, true, 1.2f)
        assertEquals(BalanceScaleKinematics.Stage.Balanced, balancedStage)
    }

    @Test
    fun testShimmerSweepRange() {
        val (start, end) = BalanceScaleKinematics.shimmerSweepRange(0.5f)
        assertEquals(0.5f * 1.8f - 0.4f - 0.28f, start, 0.001f)
        assertEquals(0.5f * 1.8f - 0.4f + 0.28f, end, 0.001f)
    }
}
