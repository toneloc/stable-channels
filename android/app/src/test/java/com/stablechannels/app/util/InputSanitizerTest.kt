package com.stablechannels.app.util

import org.junit.Assert.assertEquals
import org.junit.Test

class InputSanitizerTest {

    @Test
    fun `decimal sanitization cases`() {
        assertEquals("", InputSanitizer.decimal(""))
        assertEquals("0.", InputSanitizer.decimal("."))
        assertEquals("0.5", InputSanitizer.decimal(".5"))
        assertEquals("0", InputSanitizer.decimal("0"))
        assertEquals("0", InputSanitizer.decimal("00"))
        assertEquals("12.34", InputSanitizer.decimal("00012.34"))
        assertEquals("0.05", InputSanitizer.decimal("0.05"))
        assertEquals("12.34", InputSanitizer.decimal("12.3.4.5"))
        assertEquals("12.34", InputSanitizer.decimal("12abc.34xyz"))
        assertEquals("12.34", InputSanitizer.decimal("12.3456", maxDecimals = 2))
        assertEquals("12.345", InputSanitizer.decimal("12.3456", maxDecimals = 3))
    }
}
