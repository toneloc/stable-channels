package com.stablechannels.app.domain.send

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SendDomainPolicyTest {
    // Invoice payability

    @Test
    fun `expired wins over any earlier payment`() {
        assertEquals(InvoicePayability.EXPIRED, InvoicePayability.evaluate(true, null))
        assertEquals(
            InvoicePayability.EXPIRED,
            InvoicePayability.evaluate(true, OutboundPaymentStatus.FAILED),
        )
        assertEquals(
            InvoicePayability.EXPIRED,
            InvoicePayability.evaluate(true, OutboundPaymentStatus.SUCCEEDED),
        )
    }

    @Test
    fun `earlier payment decides`() {
        assertEquals(
            InvoicePayability.ALREADY_PAID,
            InvoicePayability.evaluate(false, OutboundPaymentStatus.SUCCEEDED),
        )
        assertEquals(
            InvoicePayability.PAYMENT_IN_PROGRESS,
            InvoicePayability.evaluate(false, OutboundPaymentStatus.PENDING),
        )
        assertEquals(
            InvoicePayability.PAYABLE,
            InvoicePayability.evaluate(false, OutboundPaymentStatus.FAILED),
        )
        assertEquals(InvoicePayability.PAYABLE, InvoicePayability.evaluate(false, null))
    }

    // Send preflight (iOS cases live in SendFlowModelTests)

    private fun input(
        isOnline: Boolean = true,
        isFeeRateReady: Boolean = true,
        amountSats: Long = 1_000,
        totalDebitSats: Long = 2_000,
        availableSats: Long = 10_000,
        sendsAllFromOnchainWallet: Boolean = false,
    ) =
        SendPreflight.Input(
            isOnline,
            isFeeRateReady,
            amountSats,
            totalDebitSats,
            availableSats,
            sendsAllFromOnchainWallet,
        )

    @Test
    fun `offline blocks before any other rule`() {
        assertEquals(
            SendError.Offline,
            SendPreflight.evaluate(input(isOnline = false, isFeeRateReady = false, amountSats = 0)),
        )
    }

    @Test
    fun `fee rate not ready blocks`() {
        assertEquals(
            SendError.FeeRateNotReady,
            SendPreflight.evaluate(input(isFeeRateReady = false)),
        )
    }

    @Test
    fun `zero amount is invalid`() {
        assertEquals(
            SendError.InvalidAmount,
            SendPreflight.evaluate(input(amountSats = 0, totalDebitSats = 0)),
        )
    }

    @Test
    fun `debit above balance or empty balance blocks`() {
        assertEquals(
            SendError.InsufficientBalance(10_000),
            SendPreflight.evaluate(input(totalDebitSats = 10_001)),
        )
        assertEquals(
            SendError.InsufficientBalance(0),
            SendPreflight.evaluate(input(totalDebitSats = 0, availableSats = 0)),
        )
    }

    @Test
    fun `debit equal to balance proceeds`() {
        assertNull(SendPreflight.evaluate(input(totalDebitSats = 10_000)))
    }

    @Test
    fun `onchain wallet send all skips amount and balance but not connectivity`() {
        assertNull(
            SendPreflight.evaluate(
                input(
                    amountSats = 0,
                    totalDebitSats = 0,
                    availableSats = 0,
                    sendsAllFromOnchainWallet = true,
                )
            )
        )
        assertEquals(
            SendError.Offline,
            SendPreflight.evaluate(input(isOnline = false, sendsAllFromOnchainWallet = true)),
        )
        assertEquals(
            SendError.FeeRateNotReady,
            SendPreflight.evaluate(input(isFeeRateReady = false, sendsAllFromOnchainWallet = true)),
        )
    }
}
