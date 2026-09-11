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
        assertEquals("0", InputSanitizer.decimal("0000"))
        assertEquals("12.34", InputSanitizer.decimal("00012.34"))
        assertEquals("0.05", InputSanitizer.decimal("0.05"))
        assertEquals("0.05", InputSanitizer.decimal("00.05"))
        assertEquals("0.000", InputSanitizer.decimal("0.000", maxDecimals = 3))
        assertEquals("100.", InputSanitizer.decimal("100."))
        assertEquals("12.34", InputSanitizer.decimal("12.3.4.5"))
        assertEquals("12.34", InputSanitizer.decimal("12abc.34xyz"))
        assertEquals("1234.56", InputSanitizer.decimal("$1,234.56"))
        assertEquals("42.50", InputSanitizer.decimal("-42.50"))
        assertEquals("50.25", InputSanitizer.decimal("  50.25  "))
        assertEquals("", InputSanitizer.decimal("abc!@#"))
        assertEquals("12.34", InputSanitizer.decimal("12.3456", maxDecimals = 2))
        assertEquals("12.345", InputSanitizer.decimal("12.3456", maxDecimals = 3))
        assertEquals("12", InputSanitizer.decimal("12.3456", maxDecimals = 0))
        assertEquals("0.12345678", InputSanitizer.decimal("0.123456789", maxDecimals = 8))
    }
}
