package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.PendingTradePayment
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.PreparedTrade
import com.stablechannels.app.services.TradeControlMessage
import com.stablechannels.app.services.TradeCorrelation
import com.stablechannels.app.services.TradeProtocol
import com.stablechannels.app.util.Constants
import java.io.File
import java.lang.reflect.InvocationTargetException
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TradeRetryPendingStateTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val channelId = "ab".repeat(32)
    private val paymentId = "cd".repeat(32)
    private val tradeId = "ef".repeat(32)

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        db = DatabaseService(context)
        db.saveChannel(channelId, "7", 100.0, 100_000L, null, 150_000L, 100_000.0)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    @Test
    fun retriedResultShowsTheTradeAsDelayed() {
        val prepared = prepareBuy()
        val row = db.recordPreparedTrade(prepared)
        assertTrue(db.attachTradePaymentId(row, paymentId))
        val state = appStateWithDatabase()
        assertTrue(state.addPendingTradePayment(paymentId, pending(prepared, row)))

        lowerReceiverBelowTheTrade()
        assertResultRetried(state, prepared)

        assertEquals("uncertain", state.pendingTradePayments.value[paymentId]?.status)
    }

    @Test
    fun retriedResultKeepsAnUnattachedTradeSoItsFeeIsStillRecognised() {
        val prepared = prepareBuy()
        // The payment id was not stored, so the row's trade_payment_id stays NULL.
        val row = db.recordPreparedTrade(prepared)
        val state = appStateWithDatabase()
        assertTrue(state.addPendingTradePayment(paymentId, pending(prepared, row)))

        lowerReceiverBelowTheTrade()
        assertResultRetried(state, prepared)
        assertNotNull(state.pendingTradePayments.value[paymentId])

        handlePaymentSuccessful(state, paymentId)
        assertEquals("fee_paid", state.pendingTradePayments.value[paymentId]?.status)
        assertEquals(100_000L, db.loadChannel("7")?.backingSats)
    }

    /** Buy $10 of $100 at $100k: backing 100,000 -> 90,000 sats. */
    private fun prepareBuy(): PreparedTrade =
        TradeProtocol.prepare(
            spendableSats = 150_000,
            sc =
                StableChannel(
                    channelId = channelId,
                    userChannelId = "7",
                    expectedUSD = USD(100.0),
                    stableReceiverBTC = Bitcoin(150_000),
                    backingSats = 100_000,
                ),
            action = "buy",
            amountUsd = 10.0,
            amountBtc = 0.0001,
            feeUsd = 0.1,
            newExpectedUsd = 90.0,
            quotePrice = 100_000.0,
            tradeId = tradeId,
        )!!

    private fun pending(prepared: PreparedTrade, row: Long) =
        PendingTradePayment(
            newExpectedUSD = prepared.newExpectedUsd,
            price = prepared.quotePrice,
            tradeDbId = row,
            action = "buy",
        )

    /** The receiver drops below the trade's 90,000 backing, so the result must be retried. */
    private fun lowerReceiverBelowTheTrade() {
        db.saveChannelPreservingBacking(channelId, "7", 100.0, null, 80_000L, 100_000.0)
    }

    private fun appStateWithDatabase() =
        AppState(context).also { state ->
            AppState::class
                .java
                .getDeclaredField("databaseService")
                .apply { isAccessible = true }
                .set(state, db)
        }

    private fun assertResultRetried(state: AppState, prepared: PreparedTrade) {
        val sync =
            TradeControlMessage.Sync(
                channelId = channelId,
                userChannelId = "7",
                expectedUsd = prepared.newExpectedUsd,
                backingSats = prepared.newBackingSats,
                syncVersion = 1,
                correlation = TradeCorrelation(tradeId, paymentId, prepared.requestHash),
            )
        try {
            AppState::class
                .java
                .getDeclaredMethod(
                    "processSignedSyncMessage",
                    TradeControlMessage::class.java,
                    String::class.java,
                    Long::class.javaPrimitiveType,
                )
                .apply { isAccessible = true }
                .invoke(state, sync, "sync-payment", TradeProtocol.RESULT_CONTROL_AMOUNT_MSAT)
            fail("the result should be retried")
        } catch (e: InvocationTargetException) {
            assertEquals("RetryableSyncException", e.targetException.javaClass.simpleName)
        }
    }

    private fun handlePaymentSuccessful(state: AppState, id: String) {
        AppState::class
            .java
            .getDeclaredMethod(
                "handlePaymentSuccessful",
                String::class.java,
                String::class.java,
                Long::class.javaObjectType,
            )
            .apply { isAccessible = true }
            .invoke(state, id, "fee-payment-hash", 0L)
    }
}
