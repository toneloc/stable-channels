package com.stablechannels.app.ui.settings.seed

import androidx.lifecycle.Lifecycle
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SeedLifecycleTest {

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
    fun `other events map to no phase`() {
        assertNull(seedLifecyclePhase(Lifecycle.Event.ON_CREATE))
        assertNull(seedLifecyclePhase(Lifecycle.Event.ON_START))
        assertNull(seedLifecyclePhase(Lifecycle.Event.ON_DESTROY))
    }
}
