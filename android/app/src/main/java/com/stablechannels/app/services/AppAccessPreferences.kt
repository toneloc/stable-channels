package com.stablechannels.app.services

import android.content.Context

/**
 * Manages App Access security preferences stored in SharedPreferences.
 *
 * Two independent toggles:
 * - App Unlock: requires biometric auth on launch and resume after 5s in background
 * - Payment Confirmation: requires biometric auth before Lightning sends
 *
 * Auth gate rules:
 * - On-chain sends ALWAYS require auth (regardless of toggle state)
 * - Seed phrase viewing ALWAYS requires auth (regardless of toggle state)
 * - Lightning sends require auth only when Payment Confirmation is enabled
 * - Disabling either toggle requires auth; enabling does not
 */
data class AppAccessPreferences(
    val appUnlockEnabled: Boolean = false,
    val paymentConfirmationEnabled: Boolean = false
)

/**
 * Utility for reading/writing App Access preferences and determining
 * when authentication is required.
 */
object AppAccessPreferencesManager {

    private const val PREFS_NAME = "app_access_prefs"
    private const val KEY_APP_UNLOCK = "app_unlock_enabled"
    private const val KEY_PAYMENT_CONFIRMATION = "payment_confirmation_enabled"

    private fun getPrefs(context: Context) =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * Reads the current App Access preferences from SharedPreferences.
     */
    fun getPreferences(context: Context): AppAccessPreferences {
        val prefs = getPrefs(context)
        return AppAccessPreferences(
            appUnlockEnabled = prefs.getBoolean(KEY_APP_UNLOCK, false),
            paymentConfirmationEnabled = prefs.getBoolean(KEY_PAYMENT_CONFIRMATION, false)
        )
    }

    /**
     * Returns whether App Unlock is enabled (require auth on launch/resume after 5s).
     */
    fun isAppUnlockEnabled(context: Context): Boolean {
        return getPrefs(context).getBoolean(KEY_APP_UNLOCK, false)
    }

    /**
     * Returns whether Payment Confirmation is enabled (require auth before Lightning sends).
     */
    fun isPaymentConfirmationEnabled(context: Context): Boolean {
        return getPrefs(context).getBoolean(KEY_PAYMENT_CONFIRMATION, false)
    }

    /**
     * Sets the App Unlock preference.
     */
    fun setAppUnlockEnabled(context: Context, enabled: Boolean) {
        getPrefs(context).edit().putBoolean(KEY_APP_UNLOCK, enabled).apply()
    }

    /**
     * Sets the Payment Confirmation preference.
     */
    fun setPaymentConfirmationEnabled(context: Context, enabled: Boolean) {
        getPrefs(context).edit().putBoolean(KEY_PAYMENT_CONFIRMATION, enabled).apply()
    }

    /**
     * Determines whether biometric authentication should be required for a send operation.
     *
     * Rules:
     * - On-chain sends ALWAYS require auth (regardless of Payment Confirmation toggle)
     * - Lightning sends require auth only when Payment Confirmation is enabled
     *
     * @param context Android context for reading preferences
     * @param isOnChain true if this is an on-chain send (splice-out or direct), false for Lightning
     * @return true if auth should be required before proceeding with the send
     */
    fun shouldRequireAuth(context: Context, isOnChain: Boolean): Boolean {
        if (isOnChain) return true
        return isPaymentConfirmationEnabled(context)
    }

    /**
     * Determines whether auth is required to change a toggle.
     *
     * Enabling a toggle does NOT require auth.
     * Disabling a toggle DOES require auth.
     *
     * @param currentlyEnabled the current state of the toggle
     * @param requestedEnabled the new state the user wants
     * @return true if auth is required for this state change
     */
    fun requiresAuthForToggleChange(currentlyEnabled: Boolean, requestedEnabled: Boolean): Boolean {
        // Auth required only when disabling (going from enabled to disabled)
        return currentlyEnabled && !requestedEnabled
    }

    /**
     * Whether auth is required to view the seed phrase.
     * Always returns true — seed phrase viewing is always gated.
     */
    fun shouldRequireAuthForSeedPhrase(): Boolean = true
}
