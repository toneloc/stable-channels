package com.stablechannels.app.util

import java.util.Date
import java.util.TimeZone
import org.junit.Assert.assertEquals
import org.junit.Test

class TimezoneRegressionTest {

    @Test
    fun `cached date formatters refresh timezone when default timezone changes`() {
        val originalDefault = TimeZone.getDefault()
        try {
            // Initialize cached formatters in UTC
            TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
            val epochZero = Date(0L)
            assertEquals("Jan 1, 12:00 AM", epochZero.shortString())
            assertEquals("Jan 1", epochZero.relativeString())

            // Switch default timezone to America/Los_Angeles (UTC-8)
            TimeZone.setDefault(TimeZone.getTimeZone("America/Los_Angeles"))
            assertEquals("Dec 31, 4:00 PM", epochZero.shortString())
            assertEquals("Dec 31", epochZero.relativeString())
        } finally {
            TimeZone.setDefault(originalDefault)
        }
    }
}
