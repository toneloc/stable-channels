package com.stablechannels.app.domain.seed

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

private val TWELVE = List(12) { "abandon" }
private val TWENTY_FOUR = List(24) { "abandon" }

class MnemonicInputTest {

    @Test
    fun `parse trims, lowercases and drops extra whitespace`() {
        assertEquals(listOf("abandon", "about"), MnemonicInput.parse("  Abandon \n\t ABOUT  "))
        assertEquals(emptyList<String>(), MnemonicInput.parse("   "))
    }

    @Test
    fun `format joins the parsed words with single spaces`() {
        assertEquals("abandon about", MnemonicInput.format(" Abandon\n about "))
    }

    @Test
    fun `word count detection shows 12 cells up to 12 words and 24 above`() {
        assertEquals(12, MnemonicInput.detectWordCount(""))
        assertEquals(12, MnemonicInput.detectWordCount(TWELVE.joinToString(" ")))
        assertEquals(24, MnemonicInput.detectWordCount((TWELVE + "x").joinToString(" ")))
    }

    @Test
    fun `only 12 or 24 words are a valid count`() {
        assertTrue(MnemonicInput.isValidWordCount(TWELVE.joinToString(" ")))
        assertTrue(MnemonicInput.isValidWordCount(TWENTY_FOUR.joinToString("  ")))
        assertFalse(MnemonicInput.isValidWordCount((TWELVE + "x").joinToString(" ")))
        assertFalse(MnemonicInput.isValidWordCount(""))
    }

    @Test
    fun `character format allows letters only and needs a word`() {
        assertTrue(MnemonicInput.hasValidCharacterFormat("Abandon about"))
        assertFalse(MnemonicInput.hasValidCharacterFormat("abandon ab0ut"))
        assertFalse(MnemonicInput.hasValidCharacterFormat("abandon ab-out"))
        assertFalse(MnemonicInput.hasValidCharacterFormat("  "))
    }

    @Test
    fun `words fill 24 fields and extra words are dropped`() {
        val fields = MnemonicInput.toFields(listOf("abandon", "about"))
        assertEquals(24, fields.size)
        assertEquals(listOf("abandon", "about") + List(22) { "" }, fields)
        assertEquals(24, MnemonicInput.toFields(TWENTY_FOUR + "x").size)
    }

    @Test
    fun `fields join back into the phrase, skipping empty cells`() {
        val fields = MnemonicInput.toFields(listOf("abandon", "", "about"))
        assertEquals("abandon about", MnemonicInput.fromFields(fields))
    }

    @Test
    fun `fields are complete with exactly 12 or 24 filled`() {
        assertTrue(MnemonicInput.isComplete(MnemonicInput.toFields(TWELVE)))
        assertTrue(MnemonicInput.isComplete(MnemonicInput.toFields(TWENTY_FOUR)))
        assertFalse(MnemonicInput.isComplete(MnemonicInput.toFields(TWELVE.drop(1))))
    }
}
