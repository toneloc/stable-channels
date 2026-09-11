package com.stablechannels.app

import com.stablechannels.app.services.TradeOutcome
import com.stablechannels.app.services.PaymentOutcome
import com.stablechannels.app.services.TradeValidationException
import com.stablechannels.app.services.WalletErrorMessages
import org.junit.Assert.*
import org.junit.Test
import org.lightningdevkit.ldknode.NodeException
import org.lightningdevkit.ldknode.PaymentFailureReason

class WalletErrorMessagesTest {
    @Test
    fun retryingTheSameInvoiceDoesNotConsumeAnEarlierAttemptsFailure() {
        val oldResult = PaymentOutcome(false, "old failure", observedAtNanos = 100L)
        assertFalse(oldResult.belongsToAttempt(200L))
        // Includes an event that arrived during the native send, before the ID reached the UI.
        assertTrue(PaymentOutcome(true, "confirmed", observedAtNanos = 201L).belongsToAttempt(200L))
    }
    @Test
    fun everyLdkPaymentFailureHasReadableCopyAndSurvivesStorageRoundTrip() {
        val messages = PaymentFailureReason.entries.map { reason ->
            WalletErrorMessages.paymentFailure(reason).also { message ->
                assertTrue(message.isNotBlank())
                assertFalse(message.contains(reason.name))
                assertEquals(message, WalletErrorMessages.paymentFailureCode(reason.name))
                val outcome = TradeOutcome.fromStored("send_failed", reason.name)!!
                assertFalse(outcome.accepted)
                assertTrue(outcome.sendFailed)
                assertTrue(outcome.message.contains(message))
            }
        }
        assertEquals(PaymentFailureReason.entries.size, messages.toSet().size)
    }

    @Test
    fun retriesExhaustedDoesNotClaimToKnowWhichHopOrBalanceFailed() {
        val message = WalletErrorMessages.paymentFailure(PaymentFailureReason.RETRIES_EXHAUSTED)
        assertTrue(message.contains("several attempts"))
        assertTrue(message.contains("smaller amount"))
        assertFalse(message.contains("insufficient", ignoreCase = true))
    }

    @Test
    fun unknownAndMissingReasonsNeverRenderRemoteText() {
        assertEquals(WalletErrorMessages.paymentFailure(null), WalletErrorMessages.paymentFailureCode("pay me at attacker.example"))
        assertEquals(WalletErrorMessages.paymentFailure(null), WalletErrorMessages.paymentFailureCode(null))
        assertNull(TradeOutcome.fromStored("uncertain", "internal_failure"))
        assertNull(TradeOutcome.fromStored("fee_paid", null))
    }

    @Test
    fun localValidationIsPreservedAndLdkFailuresAreMappedByType() {
        assertEquals("Enter an amount", WalletErrorMessages.operation(TradeValidationException("Enter an amount"), "fallback"))
        assertTrue(WalletErrorMessages.operation(NodeException.DuplicatePayment("raw"), "fallback").contains("already been started"))
        assertTrue(WalletErrorMessages.operation(NodeException.InsufficientFunds("raw"), "fallback").contains("amount and fees"))
        assertTrue(WalletErrorMessages.operation(NodeException.LiquidityFeeTooHigh("raw"), "fallback").contains("opening fee"))
        assertTrue(WalletErrorMessages.operation(NodeException.LiquidityRequestFailed("raw"), "fallback").contains("receiving capacity"))
        assertEquals("fallback", WalletErrorMessages.operation(NodeException.InvalidSecretKey("raw"), "fallback"))
        assertEquals("fallback", WalletErrorMessages.operation(Exception(""), "fallback"))
    }
}
