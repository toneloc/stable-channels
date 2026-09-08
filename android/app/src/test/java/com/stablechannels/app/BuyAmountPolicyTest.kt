package com.stablechannels.app

import com.stablechannels.app.services.BuyAmountPolicy
import com.stablechannels.app.util.usdFormatted
import org.junit.Assert.*
import org.junit.Test
import java.util.Locale

class BuyAmountPolicyTest {
    @Test fun fractionalCentBalanceDoesNotRoundMaxUp() {
        // Previously both the input and label showed $81.03, but 81.03 > 81.026.
        val maximum = BuyAmountPolicy.maximumUsd(81.026)
        assertEquals(81.02, maximum, 0.0)
        assertEquals("81.02", String.format(Locale.US, "%.2f", maximum))
        assertEquals("$81.02", maximum.usdFormatted())
        assertTrue(BuyAmountPolicy.accepts(maximum, 81.026))
        assertFalse(BuyAmountPolicy.accepts(81.03, 81.026))
    }

    @Test fun exactCentBalancesKeepTheirLastCentAndHaveNoPercentageCap() {
        for (balance in listOf(0.01, 0.29, 5.0, 81.03, 84.18, 100.0)) {
            assertEquals(balance, BuyAmountPolicy.maximumUsd(balance), 0.0)
            assertTrue(BuyAmountPolicy.accepts(balance, balance))
        }
    }

    @Test fun displayedMaximumIsAcceptedAcrossCentBoundaries() {
        for (cents in 1..10_000) {
            val exact = cents / 100.0
            for (balance in listOf(exact, Math.nextDown(exact), Math.nextUp(exact), exact + 0.006)) {
                val maximum = BuyAmountPolicy.maximumUsd(balance)
                val input = String.format(Locale.US, "%.2f", maximum).toDouble()
                assertEquals(maximum, input, 0.0)
                assertTrue("$input exceeds $balance", input <= balance)
                assertEquals(input > 0.0, BuyAmountPolicy.accepts(input, balance))
                assertFalse(BuyAmountPolicy.accepts(input + 0.01, balance))
            }
        }
    }

    @Test fun invalidBalancesFailClosed() {
        for (balance in listOf(-1.0, 0.0, Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY)) {
            assertEquals(0.0, BuyAmountPolicy.maximumUsd(balance), 0.0)
            assertFalse(BuyAmountPolicy.accepts(0.01, balance))
        }
    }

    @Test fun balancesBelowOneCentHaveNoWholeCentMaximum() {
        for (balance in listOf(Double.MIN_VALUE, 0.001, 0.009, Math.nextDown(0.01))) {
            assertEquals(0.0, BuyAmountPolicy.maximumUsd(balance), 0.0)
            assertFalse(BuyAmountPolicy.accepts(0.01, balance))
            assertFalse(BuyAmountPolicy.accepts(0.0, balance))
        }
    }

    @Test fun invalidOrOverLimitAmountsAreRejected() {
        for (amount in listOf(-1.0, 0.0, 81.025, 81.03, Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY)) {
            assertFalse(BuyAmountPolicy.accepts(amount, 81.026))
        }
    }

    @Test fun lowerLiveBalanceRejectsPreviouslySelectedMaximum() {
        val selectedMax = BuyAmountPolicy.maximumUsd(81.026)
        assertFalse(BuyAmountPolicy.accepts(selectedMax, 81.019))
        assertTrue(BuyAmountPolicy.accepts(BuyAmountPolicy.maximumUsd(81.019), 81.019))
    }
}
