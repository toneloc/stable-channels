package com.stablechannels.app.util

import android.content.Context
import android.content.SharedPreferences

/**
 * Stores a user-supplied "custom LSP" pubkey/address override in SharedPreferences.
 * Falls back to [Constants.DEFAULT_LSP_PUBKEY] / [Constants.DEFAULT_LSP_ADDRESS]
 * (stablechannels.com) when no override has been saved, keeping the app decentralized-by-default
 * while allowing advanced users to point at their own LSP.
 */
object LspPreferencesManager {
    private const val PREFS_NAME = "lsp_preferences"
    private const val KEY_CUSTOM_PUBKEY = "custom_lsp_pubkey"
    private const val KEY_CUSTOM_ADDRESS = "custom_lsp_address"

    /** Compressed pubkey: 66 hex chars, starting with 02 or 03. */
    private val PUBKEY_REGEX = Regex("^0[23][0-9a-fA-F]{64}$")

    /** host:port — hostname/IPv4 followed by a 1-5 digit port. */
    private val ADDRESS_REGEX = Regex("^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?:[0-9]{1,5}$")

    private fun prefs(context: Context): SharedPreferences =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /** True if the user has saved a custom LSP override (vs. the default stablechannels.com). */
    fun hasCustomLsp(context: Context): Boolean =
        prefs(context).contains(KEY_CUSTOM_PUBKEY) && prefs(context).contains(KEY_CUSTOM_ADDRESS)

    /** The active LSP pubkey — custom override if set, otherwise the hardcoded default. */
    fun getLspPubkey(context: Context): String =
        prefs(context).getString(KEY_CUSTOM_PUBKEY, null)?.takeIf { it.isNotBlank() }
            ?: Constants.DEFAULT_LSP_PUBKEY

    /** The active LSP address ("host:port") — custom override if set, otherwise the default. */
    fun getLspAddress(context: Context): String =
        prefs(context).getString(KEY_CUSTOM_ADDRESS, null)?.takeIf { it.isNotBlank() }
            ?: Constants.DEFAULT_LSP_ADDRESS

    fun isValidPubkey(pubkey: String): Boolean = PUBKEY_REGEX.matches(pubkey.trim())

    fun isValidAddress(address: String): Boolean = ADDRESS_REGEX.matches(address.trim())

    /**
     * Validates and persists a custom LSP pubkey/address.
     * Returns `null` on success, or a human-readable error message on validation failure.
     */
    fun saveCustomLsp(context: Context, pubkey: String, address: String): String? {
        val trimmedPubkey = pubkey.trim()
        val trimmedAddress = address.trim()

        if (!isValidPubkey(trimmedPubkey)) {
            return "Pubkey must start with 02 or 03 and be exactly 66 hex characters."
        }
        if (!isValidAddress(trimmedAddress)) {
            return "Address must be in host:port format (e.g. domain.com:9735)."
        }

        prefs(context).edit()
            .putString(KEY_CUSTOM_PUBKEY, trimmedPubkey)
            .putString(KEY_CUSTOM_ADDRESS, trimmedAddress)
            .apply()
        return null
    }

    /** Clears the custom override, restoring the default stablechannels.com LSP. */
    fun resetToDefault(context: Context) {
        prefs(context).edit()
            .remove(KEY_CUSTOM_PUBKEY)
            .remove(KEY_CUSTOM_ADDRESS)
            .apply()
    }
}
