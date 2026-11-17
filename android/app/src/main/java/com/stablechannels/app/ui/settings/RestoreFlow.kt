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

/** One channel check of [phrase]. Compared by identity, so two checks of the same phrase differ. */
internal class RestoreCheck(val phrase: String)

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

    private var activeCheck by mutableStateOf<RestoreCheck?>(null)
    // The check behind the open warning, so "Restore Anyway" restores what was checked.
    private var warnedCheck: RestoreCheck? = null
    private var isApplying by mutableStateOf(false)

    val isBusy: Boolean
        get() = activeCheck != null || isApplying

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
        warnedCheck = null
    }

    fun onBackground() {
        clearPhrase()
        error = null
        dismissWarning()
        activeCheck = null
        if (!isApplying) isOpen = false
    }

    /** The check to run for an open channel, or null after setting the word-count error. */
    fun startCheck(): RestoreCheck? {
        // A second tap while a check runs would ask the LSP twice.
        if (isBusy) return null
        if (!MnemonicInput.isValidWordCount(phrase)) {
            error = "Seed phrase must be 12 or 24 words"
            return null
        }
        error = null
        return RestoreCheck(MnemonicInput.format(phrase)).also { activeCheck = it }
    }

    /**
     * Ends the channel check: the phrase to restore now, or null when a warning opened or the check
     * is stale (the app was left, or a newer check started). A stale check leaves a newer one alone
     * without modifying active state or warnings. [onWarning] runs only when this check opens a
     * warning.
     */
    fun onChecked(
        check: RestoreCheck,
        channelExists: Boolean?,
        onWarning: (RestoreWarning) -> Unit = {},
    ): String? {
        if (check !== activeCheck) return null
        activeCheck = null
        // The phrase was edited during the check: the answer is for other words, so nothing opens.
        if (MnemonicInput.format(phrase) != check.phrase) return null
        return when (channelExists) {
            false -> check.phrase
            true -> null.also { warn(RestoreWarning.OPEN_CHANNEL, check, onWarning) }
            null -> null.also { warn(RestoreWarning.UNVERIFIED, check, onWarning) }
        }
    }

    private fun warn(
        kind: RestoreWarning,
        check: RestoreCheck,
        onWarning: (RestoreWarning) -> Unit,
    ) {
        warning = kind
        warnedCheck = check
        onWarning(kind)
    }

    fun dismissWarning() {
        warning = null
        warnedCheck = null
    }

    /** "Restore Anyway": the checked phrase to apply, or null if leaving the app cleared it. */
    fun confirmWarning(): String? = warnedCheck?.phrase.also { dismissWarning() }

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
