package com.stablechannels.app.services

import com.stablechannels.app.services.AutoSpliceIn.Step
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AutoSpliceInTest {
    private val now = 1_000_000_000L

    private fun decide(
        canSplice: Boolean = true,
        spendableSats: Long = 20_000,
        confirmations: Int? = 3,
        lastAttemptAtMs: Long? = null,
        feeRate: Long? = 2,
    ) =
        AutoSpliceIn.decide(
            canSplice,
            spendableSats,
            { confirmations },
            lastAttemptAtMs,
            now,
            { feeRate },
        )

    @Test
    fun splicesConfirmedFundsUnderTheFeeCap() {
        assertEquals(Step.SPLICE, decide())
    }

    @Test
    fun idleWithoutFundsOrWhenASpliceCannotStart() {
        assertEquals(Step.IDLE, decide(spendableSats = 0))
        assertEquals(Step.IDLE, decide(canSplice = false))
    }

    @Test
    fun waitsForThreeConfirmations() {
        assertEquals(Step.CONFIRMING, decide(confirmations = 0))
        assertEquals(Step.CONFIRMING, decide(confirmations = 2))
        assertEquals(Step.SPLICE, decide(confirmations = 3))
    }

    @Test
    fun noConfirmedTransactionDoesNotBlock() {
        assertEquals(Step.SPLICE, decide(confirmations = null))
    }

    @Test
    fun feeCapIsFiveThousandSats() {
        // 29 sat/vB is 4,930 sats; 30 sat/vB is 5,100.
        assertEquals(Step.SPLICE, decide(spendableSats = 1_000_000, feeRate = 29))
        assertEquals(Step.FEE_TOO_HIGH, decide(spendableSats = 1_000_000, feeRate = 30))
    }

    @Test
    fun feeCapIsHalfTheAmount() {
        // 2 sat/vB is 340 sats.
        assertEquals(Step.SPLICE, decide(spendableSats = 680))
        assertEquals(Step.FEE_TOO_HIGH, decide(spendableSats = 679))
    }

    @Test
    fun triesAtMostOncePerTenMinutes() {
        val justUnder = now - AutoSpliceIn.RETRY_AFTER_MS + 1
        assertEquals(Step.RETRY_LATER, decide(lastAttemptAtMs = justUnder))
        assertEquals(Step.SPLICE, decide(lastAttemptAtMs = now - AutoSpliceIn.RETRY_AFTER_MS))
    }

    @Test
    fun unknownFeeRateWaits() {
        assertEquals(Step.RETRY_LATER, decide(feeRate = null))
    }

    @Test
    fun theFeeRateIsOnlyFetchedWhenNeeded() {
        var fetches = 0
        val feeRate = {
            fetches++
            2L
        }
        AutoSpliceIn.decide(true, 20_000, { 2 }, null, now, feeRate)
        AutoSpliceIn.decide(true, 20_000, { 3 }, now, now, feeRate)
        AutoSpliceIn.decide(false, 20_000, { 3 }, null, now, feeRate)
        assertEquals(0, fetches)
    }

    @Test
    fun newestConfirmationsCountsFromTheNewestHeight() {
        assertEquals(3, AutoSpliceIn.newestConfirmations(listOf(900_000, 900_098), 900_100))
        assertEquals(1, AutoSpliceIn.newestConfirmations(listOf(900_100), 900_100))
        assertEquals(0, AutoSpliceIn.newestConfirmations(listOf(900_101), 900_100))
        assertNull(AutoSpliceIn.newestConfirmations(emptyList(), 900_100))
    }
}
