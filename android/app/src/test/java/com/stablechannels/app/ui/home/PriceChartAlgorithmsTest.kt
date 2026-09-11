package com.stablechannels.app.ui.home

import com.stablechannels.app.models.PriceRecord
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PriceChartAlgorithmsTest {

    private fun makeRecord(timestamp: Long, price: Double): PriceRecord {
        return PriceRecord(id = timestamp, price = price, source = "test", timestamp = timestamp)
    }

    @Test
    fun testLowerBound() {
        val records = listOf(
            makeRecord(100, 50000.0),
            makeRecord(200, 51000.0),
            makeRecord(300, 52000.0),
            makeRecord(400, 53000.0),
            makeRecord(500, 54000.0)
        )

        assertEquals(0, PriceChartAlgorithms.lowerBound(records, 50))
        assertEquals(0, PriceChartAlgorithms.lowerBound(records, 100))
        assertEquals(1, PriceChartAlgorithms.lowerBound(records, 150))
        assertEquals(2, PriceChartAlgorithms.lowerBound(records, 300))
        assertEquals(4, PriceChartAlgorithms.lowerBound(records, 450))
        assertEquals(4, PriceChartAlgorithms.lowerBound(records, 500))
        assertEquals(5, PriceChartAlgorithms.lowerBound(records, 550))
        assertEquals(0, PriceChartAlgorithms.lowerBound(emptyList(), 100))
    }

    @Test
    fun testMinMaxPrices() {
        val records = listOf(
            makeRecord(100, 10000.0),
            makeRecord(200, 20000.0),
            makeRecord(300, 15000.0)
        )

        val (minP, maxP) = PriceChartAlgorithms.minMaxPrices(records)
        assertEquals(9800.0, minP, 0.001)
        assertEquals(20400.0, maxP, 0.001)
    }

    @Test
    fun testLttbDownsample() {
        val records = (0..99).map { i ->
            val price = if (i == 50) 99999.0 else 50000.0 + i * 10
            makeRecord(i.toLong() * 100, price)
        }

        val targetCount = 20
        val sampled = PriceChartAlgorithms.lttbDownsample(records, targetCount)
        assertEquals(targetCount, sampled.size)
        // First and last points preserved
        assertEquals(records.first().timestamp, sampled.first().timestamp)
        assertEquals(records.last().timestamp, sampled.last().timestamp)
        // Extreme peak at 50 preserved
        assertTrue("Extreme peak should be preserved by LTTB", sampled.any { it.price == 99999.0 })
    }

    @Test
    fun testFormatYAxis() {
        assertEquals("$65K", PriceChartAlgorithms.formatYAxis(65432.0))
        assertEquals("$100K", PriceChartAlgorithms.formatYAxis(100000.0))
        assertEquals("$999", PriceChartAlgorithms.formatYAxis(999.0))
    }
}
