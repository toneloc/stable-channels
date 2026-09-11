package com.stablechannels.app.util

object MnemonicUtils {
    const val WORD_COUNT_12 = 12
    const val WORD_COUNT_24 = 24

    data class ValidationResult(
        val words: List<String>,
        val isValidWordCount: Boolean,
        val hasValidCharacterFormat: Boolean
    ) {
        val isValid: Boolean get() = isValidWordCount && hasValidCharacterFormat
        val displayString: String get() = words.joinToString(" ")
    }

    /**
     * Parse mnemonic string into list of words, trimmed and lowercased.
     * Splits across any whitespace sequence (spaces, tabs, newlines).
     */
    fun parseMnemonic(input: String): List<String> {
        return input.trim()
            .lowercase()
            .split("\\s+".toRegex())
            .filter { it.isNotEmpty() }
    }

    /**
     * Checks if a single word contains only ASCII alphabetic characters.
     */
    fun isAlphabeticWord(word: String): Boolean {
        if (word.isEmpty()) return false
        for (i in 0 until word.length) {
            val c = word[i]
            if (c !in 'a'..'z' && c !in 'A'..'Z') {
                return false
            }
        }
        return true
    }

    /**
     * Checks if the list of words has a valid BIP-39 word count (12 or 24).
     */
    fun isValidWordCount(words: List<String>): Boolean {
        val count = words.size
        return count == WORD_COUNT_12 || count == WORD_COUNT_24
    }

    /**
     * Checks if all words contain only alphabetic characters.
     */
    fun hasValidCharacterFormat(words: List<String>): Boolean {
        if (words.isEmpty()) return false
        return words.all { isAlphabeticWord(it) }
    }

    /**
     * Consolidated validation returning words and validity flags.
     */
    fun validate(mnemonic: String): ValidationResult {
        val words = parseMnemonic(mnemonic)
        return ValidationResult(
            words = words,
            isValidWordCount = isValidWordCount(words),
            hasValidCharacterFormat = hasValidCharacterFormat(words)
        )
    }

    /**
     * Formats mnemonic into standard single-space separated representation.
     */
    fun formatForDisplay(mnemonic: String): String {
        return parseMnemonic(mnemonic).joinToString(" ")
    }
}
