package com.stablechannels.app

import com.stablechannels.app.services.PaymentOutcome
import com.stablechannels.app.services.TradeOutcome
import com.stablechannels.app.ui.components.PaymentResultControls
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PaymentResultControlsTest {
    @Test
    fun unresolvedTradesHideDone() {
        for (status in listOf("pending", "fee_paid", "uncertain")) {
            assertFalse(
                status,
                PaymentResultControls.showsTradeDone(TradeOutcome.fromStored(status, null)),
            )
        }
    }

    @Test
    fun definitiveTradeResultsShowDone() {
        for (status in listOf("accepted", "rejected", "send_failed")) {
            assertTrue(
                status,
                PaymentResultControls.showsTradeDone(TradeOutcome.fromStored(status, null)),
            )
        }
    }

    @Test
    fun lightningPaymentRemainsPendingUntilAnOutcomeArrives() {
        assertTrue(PaymentResultControls.isLightningPending("payment", null, 100L))
    }

    @Test
    fun staleOutcomeCannotCompleteARetriedPayment() {
        val stale = PaymentOutcome(true, "Payment sent", observedAtNanos = 99L)
        assertTrue(PaymentResultControls.isLightningPending("payment", stale, 100L))
    }

    @Test
    fun terminalPaymentResultsDoNotDependOnDisplayCopy() {
        for (succeeded in listOf(true, false)) {
            val outcome = PaymentOutcome(succeeded, "Sending", observedAtNanos = 100L)
            assertFalse(PaymentResultControls.isLightningPending("payment", outcome, 100L))
        }
    }

    @Test
    fun nonLightningResultsAreNotTreatedAsPendingLightningPayments() {
        assertFalse(PaymentResultControls.isLightningPending(null, null, 100L))
    }
}
