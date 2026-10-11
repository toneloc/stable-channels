package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.services.TradeFailure
import com.stablechannels.app.services.TradeService
import com.stablechannels.app.services.TradeValidationException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * The service-layer guards from the #272 mislabel fix (PR #274): every refusal thrown by
 * executeBuy/executeSell must carry its own actionable message. These guards fire before
 * the node or database is touched, so real (idle) instances are safe collaborators.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TradeServiceGuardMessagesTest {
    private lateinit var service: TradeService

    @Before
    fun setUp() {
        val context: Context = RuntimeEnvironment.getApplication()
        service = TradeService(NodeService(context), DatabaseService(context))
    }

    private fun channel(expectedUsd: Double = 0.0) =
        StableChannel(expectedUSD = USD(expectedUsd))

    private fun messageOf(block: () -> Unit): String {
        try {
            block()
        } catch (e: TradeValidationException) {
            return e.message ?: ""
        }
        fail("expected TradeValidationException")
        return ""
    }

    @Test
    fun buyingWithNoStableBalanceNamesTheBalanceProblem() {
        // The original #272 report: a zero-USD buy surfaced as "Trade service unavailable".
        val message = messageOf { service.executeBuy(channel(expectedUsd = 0.0), 5.0, 0.05, 100_000.0) }
        assertEquals("That is more than your stabilized balance. Reduce the amount.", message)
    }

    @Test
    fun buyingMoreThanTheStableBalanceNamesTheBalanceProblem() {
        val message = messageOf { service.executeBuy(channel(expectedUsd = 4.64), 5.0, 0.05, 100_000.0) }
        assertEquals("That is more than your stabilized balance. Reduce the amount.", message)
    }

    @Test
    fun aColdOrBrokenPriceIsNamedOnBothPaths() {
        for (price in listOf(0.0, -1.0, Double.NaN, Double.POSITIVE_INFINITY)) {
            val buy = messageOf { service.executeBuy(channel(expectedUsd = 50.0), 5.0, 0.05, price) }
            val sell = messageOf { service.executeSell(channel(), 5.0, 0.05, price) }
            assertEquals("A fresh BTC/USD quote is required before trading.", buy)
            assertEquals("A fresh BTC/USD quote is required before trading.", sell)
        }
    }

    @Test
    fun nonsenseAmountsAreNamedOnBothPaths() {
        for (amount in listOf(0.0, -3.0, Double.NaN)) {
            val buy = messageOf { service.executeBuy(channel(expectedUsd = 50.0), amount, 0.0, 100_000.0) }
            val sell = messageOf { service.executeSell(channel(), amount, 0.0, 100_000.0) }
            assertEquals(TradeFailure.INVALID_AMOUNT.userMessage(), buy)
            assertEquals(TradeFailure.INVALID_AMOUNT.userMessage(), sell)
        }
    }

    @Test
    fun noGuardEverReportsAServiceOutage() {
        // Regression pin for #272: local refusals must never masquerade as an outage.
        val badCalls = listOf<() -> Unit>(
            { service.executeBuy(channel(expectedUsd = 0.0), 5.0, 0.05, 100_000.0) },
            { service.executeBuy(channel(expectedUsd = 50.0), Double.NaN, 0.0, 100_000.0) },
            { service.executeBuy(channel(expectedUsd = 50.0), 5.0, 0.05, 0.0) },
            { service.executeSell(channel(), -1.0, 0.0, 100_000.0) },
            { service.executeSell(channel(), 5.0, 0.05, Double.NaN) },
        )
        for (call in badCalls) {
            val message = messageOf(call).lowercase()
            assertFalse("refusal must not read as an outage: $message", "unavailable" in message)
        }
    }
}
