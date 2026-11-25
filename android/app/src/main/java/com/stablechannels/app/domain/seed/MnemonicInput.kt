package com.stablechannels.app.domain.seed

/** Typed recovery phrase parsing and basic checks; the node does full BIP39 validation. */
object MnemonicInput {
    const val WORD_COUNT_12 = 12
    const val WORD_COUNT_24 = 24

    private val WHITESPACE = "\\s+".toRegex()

    fun parse(input: String): List<String> =
        input.trim().lowercase().split(WHITESPACE).filter { it.isNotEmpty() }

    fun format(input: String): String = parse(input).joinToString(" ")

    /** How many word cells to show: 12 until the phrase grows past 12 words. */
    fun detectWordCount(input: String): Int =
        if (parse(input).size <= WORD_COUNT_12) WORD_COUNT_12 else WORD_COUNT_24

    fun isValidWordCount(input: String): Boolean = isValidCount(parse(input).size)

    fun hasValidCharacterFormat(input: String): Boolean {
        val words = parse(input)
        return words.isNotEmpty() && words.all { word -> word.all { it in 'a'..'z' } }
    }

    fun toFields(words: List<String>): List<String> =
        List(WORD_COUNT_24) { words.getOrElse(it) { "" } }

    fun fromFields(fields: List<String>): String =
        fields.filter { it.isNotEmpty() }.joinToString(" ")

    fun isComplete(fields: List<String>): Boolean = isValidCount(fields.count { it.isNotEmpty() })

    private fun isValidCount(count: Int): Boolean = count == WORD_COUNT_12 || count == WORD_COUNT_24
}
