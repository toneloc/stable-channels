package com.stablechannels.app.ui.home.balancebar

import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

enum class TradeDirection {
    BUY,
    SELL,
}

data class SliderConversionMetrics(
    val usdPct: Int,
    val btcPct: Int,
    val sideWidthDp: Dp,
    val largestPct: Int,
) {
    companion object {
        // Cache side widths by digit length to avoid measuring text on every gesture frame
        private var cachedDensityDpi: Float = -1f
        private var cachedTwoDigitWidthDp: Dp = 0.dp
        private var cachedThreeDigitWidthDp: Dp = 0.dp

        fun calculate(
            usdPct: Int,
            btcPct: Int,
            density: Density,
            measureWidthPx: (String) -> Int,
        ): SliderConversionMetrics {
            val largest = maxOf(usdPct, btcPct)
            val digits = maxOf(largest.toString().length, 2)

            val sideWidthDp =
                if (density.density == cachedDensityDpi && cachedTwoDigitWidthDp > 0.dp) {
                    if (digits > 2) cachedThreeDigitWidthDp else cachedTwoDigitWidthDp
                } else {
                    cachedDensityDpi = density.density
                    val w2Usd = measureWidthPx("00% USD")
                    val w2Btc = measureWidthPx("00% BTC")
                    val max2Px = maxOf(w2Usd, w2Btc)
                    cachedTwoDigitWidthDp = with(density) { (max2Px + 8).toDp() }

                    val w3Usd = measureWidthPx("000% USD")
                    val w3Btc = measureWidthPx("000% BTC")
                    val max3Px = maxOf(w3Usd, w3Btc)
                    cachedThreeDigitWidthDp = with(density) { (max3Px + 8).toDp() }

                    if (digits > 2) cachedThreeDigitWidthDp else cachedTwoDigitWidthDp
                }

            return SliderConversionMetrics(
                usdPct = usdPct,
                btcPct = btcPct,
                sideWidthDp = sideWidthDp,
                largestPct = largest,
            )
        }
    }
}
