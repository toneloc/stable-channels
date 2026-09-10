package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.*
import com.stablechannels.app.util.Constants
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File
import org.lightningdevkit.ldknode.Event
import org.lightningdevkit.ldknode.PaymentFailureReason

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PaymentFailureRecorderTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val channelId = "ab".repeat(32)
    private val paymentId = "cd".repeat(32)

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
        db = DatabaseService(context)
        db.saveChannel(channelId, "7", 50.0, 55_000L, null, 100_000L, 100_000.0)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
    }

    private fun prepare(tradeId: String = "ef".repeat(32)): PreparedTrade = TradeProtocol.prepare(
        StableChannel(channelId = channelId, userChannelId = "7", expectedUSD = USD(50.0),
            stableReceiverBTC = Bitcoin(100_000L), backingSats = 55_000L),
        100_000L, "sell", 10.0, 0.000099, 0.1, 59.9, 100_000.0, tradeId = tradeId
    )!!

    private fun recordFailure(pid: String = paymentId) = PaymentFailureRecorder.record(db, pid, "RETRIES_EXHAUSTED") { null }

    private fun appStateWithDatabase() = AppState(context).also { state ->
        AppState::class.java.getDeclaredField("databaseService").apply { isAccessible = true }.set(state, db)
    }

    @Test
    fun foregroundEventPublishesFailedTradeEvenBeforePendingMapRegistration() {
        val row = db.recordPreparedTrade(prepare())
        db.attachTradePaymentId(row, paymentId)
        val state = appStateWithDatabase()
        val handler = AppState::class.java.getDeclaredMethod("handleEvent", Event::class.java).apply { isAccessible = true }
        handler.invoke(state, Event.PaymentFailed(paymentId, null, PaymentFailureReason.RETRIES_EXHAUSTED))
        assertTrue(state.tradeOutcomes.value[paymentId]!!.sendFailed)
        assertTrue(state.pendingTradePayments.value.isEmpty())
        assertTrue(state.paymentOutcomes.value.isEmpty())
        assertEquals(db.terminalTradeOutcome(paymentId), state.tradeOutcomes.value[paymentId])
    }

    @Test
    fun foregroundOrdinaryFailuresAreReadableAndKeptSeparateByPaymentId() {
        val state = appStateWithDatabase()
        val handler = AppState::class.java.getDeclaredMethod("handleEvent", Event::class.java).apply { isAccessible = true }
        val otherPayment = "23".repeat(32)
        handler.invoke(state, Event.PaymentFailed(paymentId, null, PaymentFailureReason.RETRIES_EXHAUSTED))
        handler.invoke(state, Event.PaymentFailed(otherPayment, null, PaymentFailureReason.PAYMENT_EXPIRED))
        assertFalse(state.paymentOutcomes.value[paymentId]!!.succeeded)
        assertTrue(state.paymentOutcomes.value[paymentId]!!.message.contains("several attempts"))
        assertTrue(state.paymentOutcomes.value[otherPayment]!!.message.contains("expired"))
    }

    @Test
    fun outgoingLightningStartsPendingAndInvoiceRetryReusesItsHistoryRow() {
        db.recordPendingLightningPayment(paymentId, "lightning", 50_000L, 100_000.0)
        assertEquals("pending", db.getRecentPayments().single().status)
        recordFailure()
        assertEquals("failed", db.getRecentPayments().single().status)
        db.recordPendingLightningPayment(paymentId, "lightning", 60_000L, 0.0)
        val retried = db.getRecentPayments().single()
        assertEquals("pending", retried.status)
        assertEquals(60_000L, retried.amountMsat)
        assertNull(retried.amountUSD)
        db.updatePaymentStatus(paymentId, "completed", 1_000L)
        assertEquals("completed", db.getRecentPayments().single().status)
        assertEquals(1_000L, db.getRecentPayments().single().feeMsat)
    }

    @Test
    fun failedFeeIsTerminalEvenBeforeUiRegistrationAndSurvivesRestart() {
        val row = db.recordPreparedTrade(prepare())
        db.attachTradePaymentId(row, paymentId)
        val result = recordFailure()
        assertTrue(result.isTrade)
        assertTrue(result.tradeOutcome!!.sendFailed)
        assertFalse(result.tradeOutcome.accepted)
        assertFalse(db.tradeIsUnresolved(row))
        db.close()
        db = DatabaseService(context)
        assertEquals(result.tradeOutcome, db.terminalTradeOutcome(paymentId))
        assertEquals("RETRIES_EXHAUSTED", db.getRecentTrades().single().reasonCode)
        assertEquals(50.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(55_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(result, recordFailure()) // Replayed failure preserves the same result.
    }

    @Test
    fun failedUnattachedFeeIsRecoveredWithItsReason() {
        val trade = prepare()
        db.recordPreparedTrade(trade)
        val result = PaymentFailureRecorder.record(db, paymentId, "ROUTE_NOT_FOUND") { trade.feeMsat }
        assertTrue(result.isTrade)
        assertTrue(result.tradeOutcome!!.sendFailed)
        assertEquals("ROUTE_NOT_FOUND", db.getRecentTrades().single().reasonCode)
        assertEquals(result.tradeOutcome, db.terminalTradeOutcome(paymentId))
    }

    @Test
    fun unrelatedPaymentCannotFailAnUnattachedTrade() {
        val row = db.recordPreparedTrade(prepare())
        assertFalse(recordFailure().isTrade)
        assertTrue(db.tradeIsUnresolved(row))
        assertNull(db.terminalTradeOutcome(paymentId))
    }

    @Test
    fun missingResponseIsUncertainAndALateSignedRejectionStillResolvesIt() {
        val trade = prepare()
        val row = db.recordPreparedTrade(trade)
        db.attachTradePaymentId(row, paymentId)
        db.markTradeFeePaid(paymentId)
        db.markExpiredTradesUncertain(trade.expiresAt + 1)
        assertNull(db.terminalTradeOutcome(paymentId))
        assertEquals("uncertain", db.unresolvedTradePayments()[paymentId]!!.status)
        val result = db.applyTradeRejection(rejection(trade, "stale_request"))
        assertEquals(TradeControlApplyStatus.APPLIED, result.status)
        assertFalse(db.terminalTradeOutcome(paymentId)!!.sendFailed)
    }

    @Test
    fun everyServerTlvRejectionPersistsAndDoesNotChangeAllocation() {
        val codes = listOf("invalid_amount", "stale_request", "invalid_fee", "invalid_quote",
            "quote_deviation", "insufficient_capacity", "settlement_required", "unsafe_allocation", "internal_failure")
        for ((index, code) in codes.withIndex()) {
            val trade = prepare(index.toString(16).padStart(64, '0'))
            val row = db.recordPreparedTrade(trade)
            val pid = (index + 20).toString(16).padStart(64, '0')
            db.attachTradePaymentId(row, pid)
            val payload = JSONObject().apply {
                put("type", "TRADE_REJECTED_V1")
                put("channel_id", channelId)
                put("trade_id", trade.tradeId)
                put("trade_payment_id", pid)
                put("request_hash", trade.requestHash)
                put("reason_code", code)
                put("decided_at", trade.createdAt)
            }.toString()
            val bytes = JSONObject().put("payload", payload).put("signature", "test-signature").toString().toByteArray()
            val rejection = TradeProtocol.parseSignedControl(bytes, "provider") { _, sig, peer ->
                sig == "test-signature" && peer == "provider"
            } as TradeControlMessage.Rejected
            assertEquals(TradeControlApplyStatus.APPLIED, db.applyTradeRejection(rejection).status)
            val outcome = db.terminalTradeOutcome(pid)!!
            assertFalse(outcome.accepted)
            assertFalse(outcome.sendFailed)
            assertEquals(TradeProtocol.rejectionMessage(code), outcome.message)
            assertEquals(TradeControlApplyStatus.DUPLICATE, db.applyTradeRejection(rejection).status)
            // A conflicting replay cannot replace the original durable message.
            db.applyTradeRejection(rejection.copy(reasonCode = "internal_failure"))
            assertEquals(outcome, recordFailure(pid).tradeOutcome)
            assertEquals(50.0, db.loadChannel("7")!!.expectedUSD, 0.0)
            assertEquals(55_000L, db.loadChannel("7")!!.backingSats)
        }
    }

    @Test
    fun wrongCorrelationCannotRejectOrUnblockTheCurrentTrade() {
        val trade = prepare()
        val row = db.recordPreparedTrade(trade)
        db.attachTradePaymentId(row, paymentId)
        val rejection = rejection(trade, "invalid_amount")
        val other = "11".repeat(32)
        for (invalid in listOf(
            rejection.copy(channelId = other),
            rejection.copy(correlation = rejection.correlation.copy(tradeId = other)),
            rejection.copy(correlation = rejection.correlation.copy(tradePaymentId = other)),
            rejection.copy(correlation = rejection.correlation.copy(requestHash = other))
        )) assertEquals(TradeControlApplyStatus.INVALID, db.applyTradeRejection(invalid).status)
        assertTrue(db.tradeIsUnresolved(row))
        assertNull(db.terminalTradeOutcome(paymentId))
    }

    @Test
    fun aLateFailureCannotUndoAnAcceptedTradeOrAPaidFee() {
        val trade = prepare()
        val row = db.recordPreparedTrade(trade)
        db.attachTradePaymentId(row, paymentId)
        db.markTradeFeePaid(paymentId)
        assertTrue(recordFailure().isTrade)
        assertNull(recordFailure().tradeOutcome)
        assertEquals("fee_paid", db.unresolvedTradePayments()[paymentId]!!.status)
        val sync = TradeControlMessage.Sync(channelId, "7", trade.newExpectedUsd, trade.newBackingSats,
            1, TradeCorrelation(trade.tradeId, paymentId, trade.requestHash))
        assertEquals(TradeControlApplyStatus.APPLIED, db.applyCorrelatedTradeAcceptance(sync).status)
        assertTrue(recordFailure().tradeOutcome!!.accepted)
        assertEquals(trade.newExpectedUsd, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun stabilityFailureReleasesOnlyItsOwnSendMarker() {
        db.claimPendingSend(10_000L, 100_000.0)
        db.setPendingSendPaymentId(paymentId)
        assertFalse(recordFailure("10".repeat(32)).isStability)
        assertNotNull(db.loadPendingSend())
        assertTrue(recordFailure().isStability)
        assertNull(db.loadPendingSend())
        assertEquals(55_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun persistenceFailureEscapesSoTheEventWillNotBeAcknowledged() {
        val row = db.recordPreparedTrade(prepare())
        db.attachTradePaymentId(row, paymentId)
        db.writableDatabase.execSQL("CREATE TRIGGER refuse_failure BEFORE UPDATE ON trades BEGIN SELECT RAISE(ABORT, 'test'); END")
        assertThrows(android.database.sqlite.SQLiteException::class.java) { recordFailure() }
        assertTrue(db.tradeIsUnresolved(row))
        assertNull(db.terminalTradeOutcome(paymentId))
    }

    private fun rejection(trade: PreparedTrade, code: String) = TradeControlMessage.Rejected(
        channelId, TradeCorrelation(trade.tradeId, paymentId, trade.requestHash), code, trade.createdAt
    )
}
