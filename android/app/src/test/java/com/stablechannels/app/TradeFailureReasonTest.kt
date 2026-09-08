package com.stablechannels.app

import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.TradeFailure
import com.stablechannels.app.services.TradePreparation
import com.stablechannels.app.services.TradeProtocol
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Local trade refusals must name themselves. The stabilization cap already did — it throws with a
 * computed maximum. The other four returned a bare `null` and every one of them reached the user
 * as "settle the stability adjustment and retry", which is only true for one of them (issue #272).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TradeFailureReasonTest {
    private val identifier = "ab".repeat(32)

    private fun channel(
        channelId: String = identifier,
        userChannelId: String = "7",
        expectedUsd: Double = 50.0,
        receiverSats: Long = 100_000,
        backingSats: Long = 55_000
    ) = StableChannel(
        channelId = channelId,
        userChannelId = userChannelId,
        expectedUSD = USD(expectedUsd),
        stableReceiverBTC = Bitcoin(receiverSats),
        backingSats = backingSats
    )

    /** A buy, so the stabilization cap does not apply and the other refusals are observable. */
    private fun prepareBuy(
        sc: StableChannel = channel(),
        spendableSats: Long = 100_000,
        amountUsd: Double = 10.0,
        amountBtc: Double = 0.000099,
        feeUsd: Double = 0.1,
        newExpectedUsd: Double = 40.0,
        quotePrice: Double = 100_000.0
    ) = TradeProtocol.prepareOrFailure(
        sc = sc,
        spendableSats = spendableSats,
        action = "buy",
        amountUsd = amountUsd,
        amountBtc = amountBtc,
        feeUsd = feeUsd,
        newExpectedUsd = newExpectedUsd,
        quotePrice = quotePrice,
        now = 1_786_310_000L,
        tradeId = identifier
    )

    private fun reasonOf(preparation: TradePreparation): TradeFailure {
        assertTrue("expected a refusal, got $preparation", preparation is TradePreparation.Failure)
        return (preparation as TradePreparation.Failure).reason
    }

    @Test
    fun aValidTradeStillPrepares() {
        val preparation = prepareBuy()
        assertTrue("expected success, got $preparation", preparation is TradePreparation.Success)
    }

    @Test
    fun anUnusableChannelIsNotReportedAsAnAllocationProblem() {
        assertEquals(
            TradeFailure.INVALID_CHANNEL,
            reasonOf(prepareBuy(sc = channel(channelId = "not-canonical")))
        )
        assertEquals(
            TradeFailure.INVALID_CHANNEL,
            reasonOf(prepareBuy(sc = channel(userChannelId = "")))
        )
    }

    @Test
    fun anUnusableAmountIsNotReportedAsAnAllocationProblem() {
        assertEquals(TradeFailure.INVALID_AMOUNT, reasonOf(prepareBuy(amountUsd = 0.0)))
        assertEquals(TradeFailure.INVALID_AMOUNT, reasonOf(prepareBuy(amountUsd = Double.NaN)))
        assertEquals(TradeFailure.INVALID_AMOUNT, reasonOf(prepareBuy(amountBtc = -1.0)))
        assertEquals(TradeFailure.INVALID_AMOUNT, reasonOf(prepareBuy(feeUsd = Double.NaN)))
    }

    @Test
    fun anUncomputableFeeIsNotReportedAsAnAllocationProblem() {
        assertEquals(TradeFailure.FEE_UNAVAILABLE, reasonOf(prepareBuy(quotePrice = 0.0)))
    }

    @Test
    fun aFeeLargerThanTheBalanceIsNotReportedAsAnAllocationProblem() {
        assertEquals(
            TradeFailure.FEE_EXCEEDS_BALANCE,
            reasonOf(
                prepareBuy(
                    sc = channel(expectedUsd = 500.0, receiverSats = 1, backingSats = 0),
                    spendableSats = 1,
                    amountUsd = 400.0,
                    newExpectedUsd = 100.0
                )
            )
        )
    }

    @Test
    fun aGenuineAllocationProblemKeepsTheSettlementCopy() {
        val reason = reasonOf(
            prepareBuy(
                sc = channel(expectedUsd = 50.0, receiverSats = 100_000, backingSats = 0),
                newExpectedUsd = 0.0
            )
        )
        assertEquals(TradeFailure.ALLOCATION_UNAVAILABLE, reason)
        assertTrue(reason.userMessage().contains("Settle the stability adjustment"))
    }

    @Test
    fun everyReasonHasDistinctNonEmptyCopyAndOnlyOneMentionsSettlement() {
        val messages = TradeFailure.entries.map { it.userMessage() }
        assertTrue(messages.none { it.isBlank() })
        assertEquals(TradeFailure.entries.size, messages.toSet().size)
        assertEquals(1, messages.count { it.contains("Settle the stability adjustment") })
        // The outage string belongs to an absent service, never to a local refusal.
        assertTrue(messages.none { it.contains("unavailable", ignoreCase = true) })
    }

    @Test
    fun theNullableFormStillMirrorsTheTypedOne() {
        // Eight call sites still use prepare(); it must keep agreeing with prepareOrFailure().
        assertNotNull(
            TradeProtocol.prepare(
                sc = channel(), spendableSats = 100_000, action = "buy", amountUsd = 10.0,
                amountBtc = 0.000099, feeUsd = 0.1, newExpectedUsd = 40.0,
                quotePrice = 100_000.0, now = 1_786_310_000L, tradeId = identifier
            )
        )
        assertNull(
            TradeProtocol.prepare(
                sc = channel(), spendableSats = 100_000, action = "buy", amountUsd = 0.0,
                amountBtc = 0.000099, feeUsd = 0.1, newExpectedUsd = 40.0,
                quotePrice = 100_000.0, now = 1_786_310_000L, tradeId = identifier
            )
        )
    }
}
