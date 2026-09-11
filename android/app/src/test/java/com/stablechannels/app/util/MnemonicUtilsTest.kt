package com.stablechannels.app.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MnemonicUtilsTest {

    private val valid12 = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private val valid24 = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art"

    @Test
    fun `parseMnemonic trims and lowercases words`() {
        val raw = "  Abandon  ABANDON   abandon   about  "
        val parsed = MnemonicUtils.parseMnemonic(raw)
        assertEquals(listOf("abandon", "abandon", "abandon", "about"), parsed)
    }

    @Test
    fun `wordCount validation`() {
        assertTrue(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(valid12)))
        assertTrue(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(valid24)))
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic("abandon abandon")))
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic("")))
    }

    @Test
    fun `characterFormat validation`() {
        assertTrue(MnemonicUtils.hasValidCharacterFormat(MnemonicUtils.parseMnemonic(valid12)))
        assertFalse(MnemonicUtils.hasValidCharacterFormat(MnemonicUtils.parseMnemonic("abandon abandon123")))
        assertFalse(MnemonicUtils.hasValidCharacterFormat(MnemonicUtils.parseMnemonic("abandon ab!out")))
    }

    @Test
    fun `parseMnemonic handles tabs newlines and multiple spaces`() {
        val inputWithTabsAndNewlines = "abandon\tabandon\nabandon\r\nabandon abandon  abandon   abandon\tabandon\nabandon abandon abandon about"
        val words = MnemonicUtils.parseMnemonic(inputWithTabsAndNewlines)
        assertEquals(12, words.size)
        assertEquals("about", words.last())

        val result = MnemonicUtils.validate(inputWithTabsAndNewlines)
        assertTrue(result.isValid)
        assertEquals(12, result.words.size)
    }

    @Test
    fun `consolidated validation`() {
        val result12 = MnemonicUtils.validate(valid12)
        assertTrue(result12.isValid)
        assertEquals(12, result12.words.size)

        val resultInvalid = MnemonicUtils.validate("abandon abandon 123")
        assertFalse(resultInvalid.isValid)
        assertFalse(resultInvalid.isValidWordCount)
        assertFalse(resultInvalid.hasValidCharacterFormat)
    }

    @Test
    fun `word count boundary cases`() {
        fun makeWords(count: Int) = List(count) { "abandon" }.joinToString(" ")
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(1))))
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(11))))
        assertTrue(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(12))))
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(13))))
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(23))))
        assertTrue(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(24))))
        assertFalse(MnemonicUtils.isValidWordCount(MnemonicUtils.parseMnemonic(makeWords(25))))
    }

    @Test
    fun `non alphabetic character rejection`() {
        assertFalse(MnemonicUtils.isAlphabeticWord("abandon1"))
        assertFalse(MnemonicUtils.isAlphabeticWord("abandon-word"))
        assertFalse(MnemonicUtils.isAlphabeticWord("abandon_word"))
        assertFalse(MnemonicUtils.isAlphabeticWord("abandon@"))
        assertFalse(MnemonicUtils.isAlphabeticWord("abándon"))
        assertTrue(MnemonicUtils.isAlphabeticWord("abandon"))
        assertTrue(MnemonicUtils.isAlphabeticWord("ABANDON"))
    }

    @Test
    fun `formatForDisplay normalizes spaces`() {
        val messy = "  abandon\t\tabandon   \n  abandon   about  "
        assertEquals("abandon abandon abandon about", MnemonicUtils.formatForDisplay(messy))
    }
}
