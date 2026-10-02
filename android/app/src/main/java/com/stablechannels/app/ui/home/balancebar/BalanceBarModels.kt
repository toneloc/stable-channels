package com.stablechannels.app.ui.home.balancebar

import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp

enum class TradeDirection {
    BUY,
    SELL,
}

data class BalanceBarTradeEvaluation(
    val direction: TradeDirection?,
    val requestedUSD: Double,
    val clampedUSD: Double,
    val isValidTrade: Boolean,
)

data class ClampedFractionResult(
    val fraction: Float,
    val isAtSellLimit: Boolean,
)

data class SliderConversionMetrics(
    val usdPct: Int,
    val btcPct: Int,
    val sideWidthDp: Dp,
    val largestPct: Int,
) {
    companion object {
        fun calculate(
            usdPct: Int,
            btcPct: Int,
            density: Density,
            measureWidthPx: (String) -> Int,
        ): SliderConversionMetrics {
            val largest = maxOf(usdPct, btcPct)
            val digits = maxOf(largest.toString().length, 2)
            val sampleText = if (digits > 2) "000% USD" else "00% USD"
            val sampleBtc = if (digits > 2) "000% BTC" else "00% BTC"
            val widthPx = maxOf(measureWidthPx(sampleText), measureWidthPx(sampleBtc))
            val sideWidthDp = with(density) { (widthPx + 8).toDp() }

            return SliderConversionMetrics(
                usdPct = usdPct,
                btcPct = btcPct,
                sideWidthDp = sideWidthDp,
                largestPct = largest,
            )
        }
    }
}
