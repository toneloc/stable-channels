package com.stablechannels.app.domain.seed

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
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

    @Test
    fun `reveal needs a saved phrase`() {
        assertFalse(SeedAccessPolicy.canRevealPhrase(null))
        assertFalse(SeedAccessPolicy.canRevealPhrase(""))
        assertTrue(SeedAccessPolicy.canRevealPhrase("abandon ability"))
    }

    @Test
    fun `no auth prompt when app lock is off`() = runTest {
        var prompts = 0
        val opened =
            SeedAccessPolicy.authorize(
                requiresAuth = false,
                authenticate = {
                    prompts++
                    true
                },
                phaseAfter = { AppLifecyclePhase.ACTIVE },
            )
        assertTrue(opened)
        assertEquals(0, prompts)
    }

    @Test
    fun `successful auth opens the seed screen`() = runTest {
        assertTrue(
            SeedAccessPolicy.authorize(
                requiresAuth = true,
                authenticate = { true },
                phaseAfter = { AppLifecyclePhase.ACTIVE },
            )
        )
    }

    @Test
    fun `failed auth opens nothing`() = runTest {
        assertFalse(
            SeedAccessPolicy.authorize(
                requiresAuth = true,
                authenticate = { false },
                phaseAfter = { AppLifecyclePhase.ACTIVE },
            )
        )
    }

    @Test
    fun `auth reply after the app went to background opens nothing`() = runTest {
        var phase = AppLifecyclePhase.ACTIVE
        val opened =
            SeedAccessPolicy.authorize(
                requiresAuth = true,
                authenticate = {
                    phase = AppLifecyclePhase.BACKGROUND
                    true
                },
                phaseAfter = { phase },
            )
        assertFalse(opened)
    }
}
