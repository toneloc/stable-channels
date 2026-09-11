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
        assertEquals("0.00\u2009000\u2009000", 0L.btcSpacedFormatted())
        assertEquals("0.00\u2009000\u2009001", 1L.btcSpacedFormatted())
        assertEquals("0.00\u2009000\u2009010", 10L.btcSpacedFormatted())
        assertEquals("0.00\u2009000\u2009100", 100L.btcSpacedFormatted())
        assertEquals("0.00\u2009001\u2009000", 1_000L.btcSpacedFormatted())
        assertEquals("1.00\u2009000\u2009000", 100_000_000L.btcSpacedFormatted())
        assertEquals("1.23\u2009456\u2009789", 123_456_789L.btcSpacedFormatted())
        assertEquals("21000000.00\u2009000\u2009000", 2_100_000_000_000_000L.btcSpacedFormatted())
    }

    @Test
    fun `satsFormatted edge cases`() {
        assertEquals("0", 0L.satsFormatted())
        assertEquals("1", 1L.satsFormatted())
        assertEquals("999", 999L.satsFormatted())
        assertEquals("1,000", 1_000L.satsFormatted())
        assertEquals("100,000,000", 100_000_000L.satsFormatted())
        assertEquals("2,100,000,000,000,000", 2_100_000_000_000_000L.satsFormatted())
    }

    @Test
    fun `usdFormatted edge cases`() {
        assertEquals("$0.00", 0.0.usdFormatted())
        assertEquals("$0.05", 0.05.usdFormatted())
        assertEquals("$10.50", 10.5.usdFormatted())
        assertEquals("$1,000,000.00", 1_000_000.0.usdFormatted())
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
