package com.stablechannels.app.ui.settings

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** Which warning the restore guard shows before a seed-only restore. */
internal enum class RestoreWarning {
    OPEN_CHANNEL,
    UNVERIFIED,
}

/**
 * The restore dialogs' state, kept out of the view so the background races test without UI. Leaving
 * the app always drops the typed phrase and closes the dialogs, as iOS does; only a restore already
 * applying keeps its spinner, since it holds its own copy of the phrase.
 */
internal class RestoreFlow {
    var isOpen by mutableStateOf(false)
        private set

    var phrase by mutableStateOf("")
    var error by mutableStateOf<String?>(null)
        private set

    var warning by mutableStateOf<RestoreWarning?>(null)
        private set

    private var isChecking by mutableStateOf(false)
    private var isApplying by mutableStateOf(false)

    val isBusy: Boolean
        get() = isChecking || isApplying

    fun open() {
        isOpen = true
    }

    fun close() {
        if (isBusy) return
        isOpen = false
        phrase = ""
        error = null
    }

    fun onBackground() {
        phrase = ""
        error = null
        warning = null
        isChecking = false
        if (!isApplying) isOpen = false
    }

    /** The phrase to check for an open channel, or null after setting the word-count error. */
    fun startCheck(): String? {
        val input = phrase.trim()
        val words = input.split("\\s+".toRegex()).size
        if (words != 12 && words != 24) {
            error = "Seed phrase must be 12 or 24 words"
            return null
        }
        isChecking = true
        error = null
        return input
    }

    /**
     * Ends the channel check: the phrase to restore now, or null when a warning opened or the check
     * is stale (the app was left, so the phrase is gone). A stale check leaves a newer one alone.
     */
    fun onChecked(input: String, channelExists: Boolean?): String? {
        if (!isChecking || phrase.trim() != input) return null
        isChecking = false
        return when (channelExists) {
            false -> input
            true -> null.also { warning = RestoreWarning.OPEN_CHANNEL }
            null -> null.also { warning = RestoreWarning.UNVERIFIED }
        }
    }

    fun dismissWarning() {
        warning = null
    }

    /** "Restore Anyway": the phrase to apply, or null if leaving the app cleared it. */
    fun confirmWarning(): String? {
        warning = null
        return phrase.trim().ifBlank { null }
    }

    /** A blank phrase never starts a restore: the node would wipe the wallet with no seed. */
    fun startApply(input: String): Boolean {
        if (input.isBlank() || isApplying) return false
        isApplying = true
        error = null
        return true
    }

    fun onApplied() {
        isApplying = false
        close()
    }

    fun onApplyFailed(message: String) {
        isApplying = false
        error = message
    }
}
