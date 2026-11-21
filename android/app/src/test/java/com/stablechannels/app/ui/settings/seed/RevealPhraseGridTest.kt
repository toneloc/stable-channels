package com.stablechannels.app.ui.settings.seed

import org.junit.Assert.assertEquals
import org.junit.Test

class RevealPhraseGridTest {

    @Test
    fun `default and smaller text sizes use three columns`() {
        assertEquals(3, revealGridColumns(fontScale = 0.85f))
        assertEquals(3, revealGridColumns(fontScale = 1.0f))
    }

    @Test
    fun `large text sizes use two columns`() {
        assertEquals(2, revealGridColumns(fontScale = 1.15f))
        assertEquals(2, revealGridColumns(fontScale = 1.3f))
    }

    @Test
    fun `accessibility text sizes use one column`() {
        assertEquals(1, revealGridColumns(fontScale = 1.5f))
        assertEquals(1, revealGridColumns(fontScale = 2.0f))
    }

    @Test
    fun `a word that fits keeps its size`() {
        assertEquals(1.0f, nextWordScale(scale = 1.0f, overflowed = false), 0f)
    }

    @Test
    fun `an overflowing word steps down`() {
        assertEquals(0.95f, nextWordScale(scale = 1.0f, overflowed = true), 0.001f)
    }

    @Test
    fun `an overflowing word never shrinks below the floor`() {
        assertEquals(MIN_WORD_SCALE, nextWordScale(scale = 0.82f, overflowed = true), 0f)
        assertEquals(MIN_WORD_SCALE, nextWordScale(scale = MIN_WORD_SCALE, overflowed = true), 0f)
        assertEquals(0.8f, MIN_WORD_SCALE, 0f)
    }
}
