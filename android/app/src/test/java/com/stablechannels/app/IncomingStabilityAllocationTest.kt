package com.stablechannels.app

import com.stablechannels.app.services.StabilityService
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class IncomingStabilityAllocationTest {
    @Test
    fun matchesRustShortfallAndExcessRules() {
        assertEquals(
            10_000L,
            StabilityService.backingAfterIncomingStability(9_000, 10.0, 100_000.0, 1_000),
        )
        assertEquals(
            10_000L,
            StabilityService.backingAfterIncomingStability(9_000, 10.0, 100_000.0, 5_000),
        )
        assertEquals(
            10_000L,
            StabilityService.backingAfterIncomingStability(10_000, 10.0, 110_000.0, 1_000),
        )
        assertEquals(
            10_001L,
            StabilityService.backingAfterIncomingStability(10_001, 10.0, 100_000.0, 1_000),
        )
    }

    @Test
    fun rejectsInvalidBooksAndUnrepresentableTargets() {
        for (target in listOf(-1.0, Double.NaN, Double.POSITIVE_INFINITY, Double.MAX_VALUE)) {
            assertNull(
                StabilityService.backingAfterIncomingStability(9_000, target, 100_000.0, 500)
            )
        }
        assertNull(StabilityService.backingAfterIncomingStability(-1, 10.0, 100_000.0, 500))
        assertNull(StabilityService.backingAfterIncomingStability(9_000, 10.0, 100_000.0, 0))
        assertNull(StabilityService.backingAfterIncomingStability(9_000, 10.0, 100_000.0, -1))
        assertNull(
            StabilityService.backingAfterIncomingStability(
                0,
                Long.MAX_VALUE.toDouble(),
                100_000_000.0,
                1,
            )
        )
    }

    @Test
    fun capsBeforeAddingToAvoidOverflow() {
        assertEquals(
            10_000L,
            StabilityService.backingAfterIncomingStability(
                9_000,
                10.0,
                100_000.0,
                Long.MAX_VALUE,
            ),
        )
    }

    @Test
    fun usesWholeSatsEvenForSubCentTargets() {
        assertEquals(0L, StabilityService.backingAfterIncomingStability(0, 0.0009, 100_000.0, 500))
        assertEquals(9L, StabilityService.backingAfterIncomingStability(0, 0.0099, 100_000.0, 500))
        assertEquals(
            20L,
            StabilityService.backingAfterIncomingStability(20, 0.0099, 100_000.0, 500),
        )
    }
}
