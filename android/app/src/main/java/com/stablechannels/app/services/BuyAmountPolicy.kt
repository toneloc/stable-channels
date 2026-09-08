package com.stablechannels.app.services

import java.math.BigDecimal
import java.math.RoundingMode

/** USD -> BTC uses the available USD balance, not the BTC -> USD stabilization cap. */
object BuyAmountPolicy {
    fun maximumUsd(balanceUsd: Double): Double {
        if (!balanceUsd.isFinite() || balanceUsd <= 0.0) return 0.0
        // Never round Max above the balance. Decimal rounding also avoids dropping an
        // extra cent for exact amounts such as 0.29, whose Double * 100 is below 29.
        return BigDecimal.valueOf(balanceUsd).setScale(2, RoundingMode.DOWN).toDouble()
    }

    fun accepts(amountUsd: Double, balanceUsd: Double): Boolean =
        amountUsd.isFinite() && amountUsd > 0.0 && amountUsd <= maximumUsd(balanceUsd)
}
