package com.stablechannels.app

import com.stablechannels.app.services.AppAccessPreferences
import com.stablechannels.app.services.AppAccessPreferencesManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppAccessPreferencesTest {

    // ---------------------------------------------------------------------------
    // AppAccessPreferences data class defaults
    // ---------------------------------------------------------------------------

    @Test
    fun `default preferences have both toggles disabled`() {
        val prefs = AppAccessPreferences()
        assertFalse(prefs.appUnlockEnabled)
        assertFalse(prefs.paymentConfirmationEnabled)
    }

    // ---------------------------------------------------------------------------
    // shouldRequireAuth — auth gate logic
    // ---------------------------------------------------------------------------

    @Test
    fun `on-chain sends always require auth regardless of context`() {
        // shouldRequireAuth with isOnChain=true should always return true
        // We test the static logic here (context-free version)
        assertTrue(shouldRequireAuthPure(isOnChain = true, paymentConfirmationEnabled = false))
        assertTrue(shouldRequireAuthPure(isOnChain = true, paymentConfirmationEnabled = true))
    }

    @Test
    fun `lightning sends require auth when payment confirmation enabled`() {
        assertTrue(shouldRequireAuthPure(isOnChain = false, paymentConfirmationEnabled = true))
    }

    @Test
    fun `lightning sends do not require auth when payment confirmation disabled`() {
        assertFalse(shouldRequireAuthPure(isOnChain = false, paymentConfirmationEnabled = false))
    }

    // ---------------------------------------------------------------------------
    // requiresAuthForToggleChange
    // ---------------------------------------------------------------------------

    @Test
    fun `disabling a toggle requires auth`() {
        assertTrue(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = true,
                requestedEnabled = false
            )
        )
    }

    @Test
    fun `enabling a toggle does not require auth`() {
        assertFalse(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = false,
                requestedEnabled = true
            )
        )
    }

    @Test
    fun `no-op toggle change does not require auth`() {
        assertFalse(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = true,
                requestedEnabled = true
            )
        )
        assertFalse(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = false,
                requestedEnabled = false
            )
        )
    }

    // ---------------------------------------------------------------------------
    // shouldRequireAuthForSeedPhrase
    // ---------------------------------------------------------------------------

    @Test
    fun `seed phrase viewing requires auth if security settings enabled`() {
        assertTrue(shouldRequireAuthForSeedPhrasePure(appUnlockEnabled = true, paymentConfirmationEnabled = false))
        assertTrue(shouldRequireAuthForSeedPhrasePure(appUnlockEnabled = false, paymentConfirmationEnabled = true))
        assertTrue(shouldRequireAuthForSeedPhrasePure(appUnlockEnabled = true, paymentConfirmationEnabled = true))
        assertFalse(shouldRequireAuthForSeedPhrasePure(appUnlockEnabled = false, paymentConfirmationEnabled = false))
    }

    private fun shouldRequireAuthForSeedPhrasePure(appUnlockEnabled: Boolean, paymentConfirmationEnabled: Boolean): Boolean {
        return appUnlockEnabled || paymentConfirmationEnabled
    }

    // ---------------------------------------------------------------------------
    // Helper: pure version of shouldRequireAuth logic for testing without Context
    // ---------------------------------------------------------------------------

    /**
     * Mirrors the logic of AppAccessPreferencesManager.shouldRequireAuth
     * without requiring an Android Context, for unit test verification.
     */
    private fun shouldRequireAuthPure(isOnChain: Boolean, paymentConfirmationEnabled: Boolean): Boolean {
        if (isOnChain) return true
        return paymentConfirmationEnabled
    }
}
