package com.stablechannels.app.ui.settings

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.stablechannels.app.domain.seed.MnemonicInput

/** Which warning the restore guard shows before a seed-only restore. */
internal enum class RestoreWarning {
    OPEN_CHANNEL,
    UNVERIFIED,
}

private val EMPTY_FIELDS = MnemonicInput.toFields(emptyList())

/**
 * The restore sheet's state, kept out of the view so the background races test without UI. Leaving
 * the app always drops the typed phrase and closes the sheet, as iOS does; only a restore already
 * applying keeps its spinner, since it holds its own copy of the phrase.
 */
internal class RestoreFlow {
    var isOpen by mutableStateOf(false)
        private set

    var phrase by mutableStateOf("")
        private set

    var wordFields by mutableStateOf(EMPTY_FIELDS)
        private set

    /** The cells mirror a non-empty phrase read-only; typing in a cell keeps them editable. */
    var isWordFieldsReadOnly by mutableStateOf(false)
        private set

    var error by mutableStateOf<String?>(null)
        private set

    var warning by mutableStateOf<RestoreWarning?>(null)
        private set

    private var isChecking by mutableStateOf(false)
    private var isApplying by mutableStateOf(false)

    val isBusy: Boolean
        get() = isChecking || isApplying

    val canRestore: Boolean
        get() =
            !isBusy &&
                MnemonicInput.isComplete(wordFields) &&
                MnemonicInput.hasValidCharacterFormat(phrase)

    /** Shown while the words hold more than letters, e.g. a numbered paste ("1. abandon"). */
    val formatError: String?
        get() =
            if (phrase.isBlank() || MnemonicInput.hasValidCharacterFormat(phrase)) null
            else "Enter only the words, without numbers or punctuation"

    fun open() {
        isOpen = true
    }

    fun onPhraseChanged(text: String) {
        phrase = text
        wordFields = MnemonicInput.toFields(MnemonicInput.parse(text))
        isWordFieldsReadOnly = text.isNotEmpty()
    }

    /** A phrase pasted into one cell spreads across the cells from that cell on, as on iOS. */
    fun onWordChanged(index: Int, word: String) {
        val pasted = MnemonicInput.parse(word)
        wordFields =
            if (pasted.size > 1) MnemonicInput.toFields(wordFields.take(index) + pasted)
            else wordFields.toMutableList().also { it[index] = word.filterNot(Char::isWhitespace) }
        phrase = MnemonicInput.fromFields(wordFields)
        isWordFieldsReadOnly = false
    }

    fun close() {
        if (isBusy) return
        isOpen = false
        clearPhrase()
        error = null
    }

    fun onBackground() {
        clearPhrase()
        error = null
        warning = null
        isChecking = false
        if (!isApplying) isOpen = false
    }

    /** The phrase to check for an open channel, or null after setting the word-count error. */
    fun startCheck(): String? {
        if (!MnemonicInput.isValidWordCount(phrase)) {
            error = "Seed phrase must be 12 or 24 words"
            return null
        }
        isChecking = true
        error = null
        return MnemonicInput.format(phrase)
    }

    /**
     * Ends the channel check: the phrase to restore now, or null when a warning opened or the check
     * is stale (the app was left, so the phrase is gone). A stale check leaves a newer one alone.
     */
    fun onChecked(input: String, channelExists: Boolean?): String? {
        if (!isChecking || MnemonicInput.format(phrase) != input) return null
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
        return MnemonicInput.format(phrase).ifBlank { null }
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

    private fun clearPhrase() {
        phrase = ""
        wordFields = EMPTY_FIELDS
        isWordFieldsReadOnly = false
    }
}
