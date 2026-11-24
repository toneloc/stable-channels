package com.stablechannels.app.ui.settings.seed

import androidx.lifecycle.Lifecycle
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import com.stablechannels.app.domain.seed.SeedAccessPolicy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SeedLifecycleTest {

    private fun phaseAfter(vararg events: Lifecycle.Event): AppLifecyclePhase =
        events.fold(AppLifecyclePhase.ACTIVE) { phase, event -> seedLifecyclePhase(event) ?: phase }

    @Test
    fun `pause is inactive and stop is background`() {
        assertEquals(AppLifecyclePhase.INACTIVE, seedLifecyclePhase(Lifecycle.Event.ON_PAUSE))
        assertEquals(AppLifecyclePhase.BACKGROUND, seedLifecyclePhase(Lifecycle.Event.ON_STOP))
    }

    @Test
    fun `resume is active`() {
        assertEquals(AppLifecyclePhase.ACTIVE, seedLifecyclePhase(Lifecycle.Event.ON_RESUME))
    }

    @Test
    fun `start is inactive`() {
        assertEquals(AppLifecyclePhase.INACTIVE, seedLifecyclePhase(Lifecycle.Event.ON_START))
    }

    @Test
    fun `other events map to no phase`() {
        assertNull(seedLifecyclePhase(Lifecycle.Event.ON_CREATE))
        assertNull(seedLifecyclePhase(Lifecycle.Event.ON_DESTROY))
    }

    @Test
    fun `a PIN screen that stops the app lets the reply open once the app is back`() {
        val phase =
            phaseAfter(Lifecycle.Event.ON_PAUSE, Lifecycle.Event.ON_STOP, Lifecycle.Event.ON_START)
        assertTrue(SeedAccessPolicy.canOpenSeedScreen(after = phase))
    }

    @Test
    fun `a reply that lands while the app is still away opens nothing`() {
        val phase = phaseAfter(Lifecycle.Event.ON_PAUSE, Lifecycle.Event.ON_STOP)
        assertFalse(SeedAccessPolicy.canOpenSeedScreen(after = phase))
    }
}
