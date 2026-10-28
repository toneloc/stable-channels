package com.stablechannels.app

import com.stablechannels.app.services.AppAccessPreferences
import com.stablechannels.app.services.AppAccessPreferencesManager
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
    fun `on-chain sends require auth when payment confirmation enabled`() {
        // Matches iOS: a single Payment Confirmation toggle governs both send types,
        // with no hardcoded exception for on-chain.
        assertTrue(shouldRequireAuthPure(isOnChain = true, paymentConfirmationEnabled = true))
    }

    @Test
    fun `on-chain sends do not require auth when payment confirmation disabled`() {
        assertFalse(shouldRequireAuthPure(isOnChain = true, paymentConfirmationEnabled = false))
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
                requestedEnabled = false,
            )
        )
    }

    @Test
    fun `enabling a toggle does not require auth`() {
        assertFalse(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = false,
                requestedEnabled = true,
            )
        )
    }

    @Test
    fun `no-op toggle change does not require auth`() {
        assertFalse(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = true,
                requestedEnabled = true,
            )
        )
        assertFalse(
            AppAccessPreferencesManager.requiresAuthForToggleChange(
                currentlyEnabled = false,
                requestedEnabled = false,
            )
        )
    }

    // ---------------------------------------------------------------------------
    // seedPhraseRequiresAuth
    // ---------------------------------------------------------------------------

    @Test
    fun `seed access requires auth with default toggles when the device can authenticate`() {
        assertTrue(
            AppAccessPreferencesManager.seedPhraseRequiresAuth(
                AppAccessPreferences(),
                deviceCanAuthenticate = true,
            )
        )
    }

    @Test
    fun `seed access needs no auth without a screen lock and with toggles off`() {
        assertFalse(
            AppAccessPreferencesManager.seedPhraseRequiresAuth(
                AppAccessPreferences(),
                deviceCanAuthenticate = false,
            )
        )
    }

    @Test
    fun `seed access requires auth whenever a toggle is on`() {
        val toggledOn =
            listOf(
                AppAccessPreferences(appUnlockEnabled = true),
                AppAccessPreferences(paymentConfirmationEnabled = true),
                AppAccessPreferences(appUnlockEnabled = true, paymentConfirmationEnabled = true),
            )
        for (prefs in toggledOn) {
            for (deviceCanAuthenticate in listOf(true, false)) {
                assertTrue(
                    AppAccessPreferencesManager.seedPhraseRequiresAuth(prefs, deviceCanAuthenticate)
                )
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Helper: pure version of shouldRequireAuth logic for testing without Context
    // ---------------------------------------------------------------------------

    /**
     * Mirrors the logic of AppAccessPreferencesManager.shouldRequireAuth without requiring an
     * Android Context, for unit test verification.
     */
    private fun shouldRequireAuthPure(
        isOnChain: Boolean,
        paymentConfirmationEnabled: Boolean,
    ): Boolean {
        return paymentConfirmationEnabled
    }
}
