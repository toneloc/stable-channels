package com.stablechannels.app.domain.seed

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SeedAccessPolicyTest {

    @Test
    fun `only background closes seed screens`() {
        assertTrue(SeedAccessPolicy.shouldCloseSeedScreens(AppLifecyclePhase.BACKGROUND))
        assertFalse(SeedAccessPolicy.shouldCloseSeedScreens(AppLifecyclePhase.INACTIVE))
        assertFalse(SeedAccessPolicy.shouldCloseSeedScreens(AppLifecyclePhase.ACTIVE))
    }

    @Test
    fun `late auth reply opens seed screen only outside background`() {
        assertTrue(SeedAccessPolicy.canOpenSeedScreen(after = AppLifecyclePhase.ACTIVE))
        assertTrue(SeedAccessPolicy.canOpenSeedScreen(after = AppLifecyclePhase.INACTIVE))
        assertFalse(SeedAccessPolicy.canOpenSeedScreen(after = AppLifecyclePhase.BACKGROUND))
    }
}
