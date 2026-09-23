package com.stablechannels.app.ui.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins PendingAmountFormatter (#316 follow-up review finding): PendingRow's amount and the "Move
 * ... to Lightning" label switched from BTC to USD display, but showed nothing at all when the
 * price feed was momentarily unavailable (btcPrice <= 0) instead of a graceful fallback. Both now
 * fall back to a BTC figure in that case rather than silently dropping the amount.
 *
 * Compose has no unit-test harness in this project, so the formatting was pulled out of the
 * Composables into this plain object specifically to be testable here (see
 * BalanceScaleKinematicsTest for the same extraction pattern elsewhere in the codebase).
 */
class PendingAmountFormatterTest {

    @Test
    fun formatsAsUsdWhenPriceIsAvailable() {
        val text = PendingAmountFormatter.amountText(amountSats = 100_000L, btcPrice = 50_000.0)
        assertTrue(text!!.startsWith("$"))
    }

    @Test
    fun fallsBackToBtcWhenPriceIsUnavailable() {
        val text = PendingAmountFormatter.amountText(amountSats = 100_000L, btcPrice = 0.0)
        assertTrue(text!!.endsWith("BTC"))
    }

    @Test
    fun fallsBackToBtcWhenPriceIsNegative() {
        val text = PendingAmountFormatter.amountText(amountSats = 100_000L, btcPrice = -1.0)
        assertTrue(text!!.endsWith("BTC"))
    }

    @Test
    fun nullAmountSatsReturnsNull() {
        assertNull(PendingAmountFormatter.amountText(amountSats = null, btcPrice = 50_000.0))
    }

    @Test
    fun moveToLightningLabelUsesUsdWhenPriceIsAvailable() {
        val label =
            PendingAmountFormatter.moveToLightningLabel(
                spendableSats = 100_000L,
                btcPrice = 50_000.0,
            )
        assertTrue(label.contains("$"))
        assertTrue(label.endsWith("to Lightning"))
    }

    @Test
    fun moveToLightningLabelFallsBackToBtcWhenPriceIsUnavailable() {
        val label =
            PendingAmountFormatter.moveToLightningLabel(spendableSats = 100_000L, btcPrice = 0.0)
        assertTrue(label.contains("BTC"))
        assertTrue(label.endsWith("to Lightning"))
        assertEquals(false, label.contains("$"))
    }
}
