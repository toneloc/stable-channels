package com.stablechannels.app.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Date

class ExtensionsTest {

    @Test
    fun `satsFormatted formats with commas`() {
        val sats = 1_234_567L
        assertEquals("1,234,567", sats.satsFormatted())
    }

    @Test
    fun `usdFormatted formats currency`() {
        val usd = 1234.56
        assertEquals("$1,234.56", usd.usdFormatted())
    }

    @Test
    fun `btcSpacedFormatted groups decimals with thin spaces`() {
        val sats = 19_0079L
        assertEquals("0.00\u2009190\u2009079", sats.btcSpacedFormatted())
    }

    @Test
    fun `date formatters return non-empty strings`() {
        val now = Date()
        assertTrue(now.relativeString().isNotEmpty())
        assertTrue(now.shortString().isNotEmpty())
    }

    @Test
    fun `AppFormatters direct methods format accurately`() {
        assertEquals("50,000", AppFormatters.formatSats(50_000L))
        assertEquals("$50.50", AppFormatters.formatUsd(50.5))
        assertTrue(AppFormatters.formatShortDateTime(Date()).isNotEmpty())
    }
}
