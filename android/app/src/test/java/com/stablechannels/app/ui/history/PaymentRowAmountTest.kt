package com.stablechannels.app.ui.history

import com.stablechannels.app.models.PaymentRecord
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PaymentRowAmountTest {
    private fun payment(direction: String, amountUSD: Double?, btcPrice: Double? = null) =
        PaymentRecord(
            id = 1,
            paymentId = "p",
            paymentType = "lightning",
            direction = direction,
            amountMsat = 100_000_000,
            amountUSD = amountUSD,
            btcPrice = btcPrice,
            counterparty = null,
            status = "completed",
            createdAt = 0,
        )

    @Test
    fun receivedIsPlusAndSentIsMinus() {
        val received = payment("received", 12.5).signedAmountText(0.0)
        val sent = payment("sent", 12.5).signedAmountText(0.0)

        assertEquals("+$12.50", received)
        assertEquals("-$12.50", sent)
    }

    @Test
    fun usesCurrentPriceWhenUsdAndStoredPriceMissing() {
        assertEquals("-$50.00", payment("sent", null).signedAmountText(50_000.0))
    }

    @Test
    fun fallsBackToBtcWhenNoPriceAvailable() {
        val text = payment("received", null).signedAmountText(0.0)

        assertTrue(text.startsWith("+") && text.endsWith("BTC"))
    }

    @Test
    fun recentActivityShowsOneRowWhenCrampedAndThreeWhenRoomy() {
        assertEquals(1, com.stablechannels.app.ui.home.recentActivityRowCount(40f))
        assertEquals(1, com.stablechannels.app.ui.home.recentActivityRowCount(-100f))
        assertEquals(2, com.stablechannels.app.ui.home.recentActivityRowCount(160f))
        assertEquals(3, com.stablechannels.app.ui.home.recentActivityRowCount(220f))
        assertEquals(4, com.stablechannels.app.ui.home.recentActivityRowCount(400f))
    }
}
