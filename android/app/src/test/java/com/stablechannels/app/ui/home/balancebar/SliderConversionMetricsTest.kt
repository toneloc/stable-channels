package com.stablechannels.app.ui.home.balancebar

import androidx.compose.ui.unit.Density
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SliderConversionMetricsTest {

    private val testDensity = Density(density = 2f, fontScale = 1f)
    // Simulated monospace/tabular width: 8px per character
    private val mockMeasureWidth: (String) -> Int = { it.length * 8 }

    @Test
    fun symmetricSideWidthFor55USD45BTC() {
        val m5545 =
            SliderConversionMetrics.calculate(
                usdPct = 55,
                btcPct = 45,
                density = testDensity,
                measureWidthPx = mockMeasureWidth,
            )
        val m4555 =
            SliderConversionMetrics.calculate(
                usdPct = 45,
                btcPct = 55,
                density = testDensity,
                measureWidthPx = mockMeasureWidth,
            )

        assertEquals(55, m5545.largestPct)
        assertEquals(55, m4555.largestPct)
        assertEquals(m5545.sideWidthDp, m4555.sideWidthDp)
        assertTrue(m5545.sideWidthDp.value > 0f)
    }

    @Test
    fun antiFlickerTwoDigitStability() {
        val baseMetrics =
            SliderConversionMetrics.calculate(
                usdPct = 50,
                btcPct = 50,
                density = testDensity,
                measureWidthPx = mockMeasureWidth,
            )

        val testSplits =
            listOf(
                55 to 45,
                60 to 40,
                75 to 25,
                88 to 12,
                99 to 1,
                11 to 89,
            )

        for ((usd, btc) in testSplits) {
            val m =
                SliderConversionMetrics.calculate(
                    usdPct = usd,
                    btcPct = btc,
                    density = testDensity,
                    measureWidthPx = mockMeasureWidth,
                )
            assertEquals(
                "Side width for $usd/$btc must match base 50/50 width to prevent jitter",
                baseMetrics.sideWidthDp,
                m.sideWidthDp,
            )
        }
    }

    @Test
    fun boundaryExpansionFor100Percent() {
        val m100 =
            SliderConversionMetrics.calculate(
                usdPct = 100,
                btcPct = 0,
                density = testDensity,
                measureWidthPx = mockMeasureWidth,
            )
        val m99 =
            SliderConversionMetrics.calculate(
                usdPct = 99,
                btcPct = 1,
                density = testDensity,
                measureWidthPx = mockMeasureWidth,
            )

        assertEquals(100, m100.largestPct)
        assertTrue(m100.sideWidthDp >= m99.sideWidthDp)
    }
}
