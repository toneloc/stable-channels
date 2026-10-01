package com.stablechannels.app

import android.content.Context
import android.database.sqlite.SQLiteException
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.TradeControlApplyStatus
import com.stablechannels.app.services.TradeControlMessage
import com.stablechannels.app.services.TradeCorrelation
import com.stablechannels.app.services.TradeProtocol
import com.stablechannels.app.util.Constants
import java.io.File
import java.util.Date
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class BalanceCorrectionTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val channelId = "ab".repeat(32)
    private val txid = "cd".repeat(32)
    private val price = 100_000.0

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        db = DatabaseService(context)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    private fun save(expected: Double, backing: Long, receiver: Long) =
        db.saveChannel(channelId, "7", expected, backing, null, receiver, price)

    private fun sync(expected: Double, version: Long = 1) =
        TradeControlMessage.Sync(
            channelId,
            "lsp-has-a-different-id",
            expected,
            999_999L,
            version,
            null,
        )

    private fun assertBooks(expected: Double, backing: Long, version: Long) {
        val row = db.loadChannel("7")!!
        assertEquals(expected, row.expectedUSD, 0.000000001)
        assertEquals(backing, row.backingSats)
        assertEquals(version, row.syncVersion)
    }

    @Test
    fun correctionClampsAnOverCapacityAllocationAndSurvivesReplayAndRestart() {
        save(50.0, 55_000L, 58_000L)
        assertNull(TradeProtocol.tradeBackingAfterDelta(58_000L, 55_000L, 50.0, 60.0, price))
        val result = db.applyUncorrelatedSyncIfNewer(sync(60.0), price)
        assertEquals(TradeControlApplyStatus.APPLIED, result.status)
        assertEquals(58_000L, result.localBackingSats)
        assertEquals(999_999L, result.peerBackingSats)
        assertBooks(60.0, 58_000L, 1)
        db.close()
        db = DatabaseService(context)
        assertEquals(
            TradeControlApplyStatus.DUPLICATE,
            db.applyUncorrelatedSyncIfNewer(sync(60.0), price).status,
        )
        assertEquals(
            TradeControlApplyStatus.DUPLICATE,
            db.applyUncorrelatedSyncIfNewer(sync(70.0, version = 0), price).status,
        )
        assertBooks(60.0, 58_000L, 1)
    }

    @Test
    fun correctionRestoresATargetJustAboveTheWalletsCapacity() {
        save(0.0, 0L, 100_000L)
        assertNull(TradeProtocol.tradeBackingAfterDelta(100_000L, 0L, 0.0, 100.001, price))
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyUncorrelatedSyncIfNewer(sync(100.001), price).status,
        )
        assertBooks(100.001, 100_000L, 1)
    }

    @Test
    fun correlatedTradeAcceptanceStillRequiresCapacityForItsStoredAllocation() {
        save(50.0, 55_000L, 100_000L)
        val trade =
            TradeProtocol.prepare(
                StableChannel(
                    channelId = channelId,
                    userChannelId = "7",
                    expectedUSD = USD(50.0),
                    stableReceiverBTC = Bitcoin(100_000L),
                    backingSats = 55_000L,
                ),
                100_000L,
                "sell",
                10.0,
                0.000099,
                0.1,
                59.9,
                price,
                tradeId = "ef".repeat(32),
            )!!
        val tradeRow = db.recordPreparedTrade(trade)
        val paymentId = "12".repeat(32)
        db.attachTradePaymentId(tradeRow, paymentId)
        val message =
            TradeControlMessage.Sync(
                channelId,
                "7",
                trade.newExpectedUsd,
                999_999L,
                1,
                TradeCorrelation(trade.tradeId, paymentId, trade.requestHash),
            )
        save(50.0, 55_000L, 58_000L)
        assertEquals(
            TradeControlApplyStatus.RETRY,
            db.applyCorrelatedTradeAcceptance(message).status,
        )
        assertBooks(50.0, 55_000L, 0)
        assertTrue(db.tradeIsUnresolved(tradeRow))
        save(50.0, 55_000L, 100_000L)
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyCorrelatedTradeAcceptance(message).status,
        )
        assertBooks(trade.newExpectedUsd, trade.newBackingSats, 1)
        assertFalse(db.tradeIsUnresolved(tradeRow))
    }

    @Test
    fun correctionPreservesLocalDriftAndUsesCumulativeFloorsForTheDelta() {
        save(70.0, 65_000L, 100_000L)
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyUncorrelatedSyncIfNewer(sync(60.0), price).status,
        )
        // At this floating-point price, floor(60 / price * satsPerBtc) is 59,999.
        assertBooks(60.0, 54_999L, 1)
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyUncorrelatedSyncIfNewer(sync(60.0, version = 2), 90_000.0).status,
        )
        assertBooks(60.0, 54_999L, 2)
    }

    @Test
    fun correctionStillRetriesWhenNoPositiveBackingCanBeDerived() {
        save(50.0, 1_000L, 100_000L)
        assertEquals(
            TradeControlApplyStatus.RETRY,
            db.applyUncorrelatedSyncIfNewer(sync(40.0), price).status,
        )
        assertBooks(50.0, 1_000L, 0)
        save(0.0, 0L, 0L)
        assertEquals(
            TradeControlApplyStatus.RETRY,
            db.applyUncorrelatedSyncIfNewer(sync(40.0), price).status,
        )
        assertBooks(0.0, 0L, 0)
    }

    @Test
    fun invalidOrUnrepresentablePricesNeverAdvanceTheVersion() {
        save(50.0, 55_000L, 58_000L)
        for (badPrice in listOf(0.0, -1.0, Double.NaN, Double.POSITIVE_INFINITY)) {
            assertEquals(
                TradeControlApplyStatus.INVALID,
                db.applyUncorrelatedSyncIfNewer(sync(60.0), badPrice).status,
            )
            assertBooks(50.0, 55_000L, 0)
        }
        assertEquals(
            TradeControlApplyStatus.RETRY,
            db.applyUncorrelatedSyncIfNewer(sync(60.0), Double.MIN_VALUE).status,
        )
        assertBooks(50.0, 55_000L, 0)
        // Adding the derived delta would overflow Long before it is clamped.
        save(0.0, Long.MAX_VALUE - 2, Long.MAX_VALUE - 1)
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyUncorrelatedSyncIfNewer(sync(10.0), price).status,
        )
        assertBooks(10.0, Long.MAX_VALUE - 1, 1)
    }

    @Test
    fun failedCorrectionRollsBackItsBooksAndVersionThenRetries() {
        save(50.0, 55_000L, 58_000L)
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_sync BEFORE UPDATE ON channels BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        assertThrows(SQLiteException::class.java) {
            db.applyUncorrelatedSyncIfNewer(sync(60.0), price)
        }
        assertBooks(50.0, 55_000L, 0)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_sync")
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyUncorrelatedSyncIfNewer(sync(60.0), price).status,
        )
        assertBooks(60.0, 58_000L, 1)
    }

    @Test
    fun spliceCompletionUsesCommittedCorrectionEvenWithAnOlderMemorySnapshot() {
        save(10.0, 10_000L, 9_000L)
        val state = stateWithBooks(10.0, 10_000L, 9_000L)
        val rowId = pendingSplice()
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            db.applyUncorrelatedSyncIfNewer(sync(8.0), price).status,
        )
        // The database was corrected while memory still holds the pre-correction books.
        assertEquals("COMPLETED", completeSplice(state, rowId))
        assertBooks(8.0, 8_000L, 1)
        assertEquals(8.0, state.stableChannel.value.expectedUSD.amount, 0.0)
        assertEquals(8_000L, state.stableChannel.value.backingSats)
        assertEquals(1_000L, state.stableChannel.value.nativeChannelBTC.sats)
        assertFalse(db.hasPendingSpliceFor(txid))
        assertTrue(processSync(state, sync(8.0)))
        assertBooks(8.0, 8_000L, 1)
    }

    @Test
    fun signedCorrectionWaitsForSpliceAccountingThenPublishesAndCachesTheLatestBooks() {
        save(10.0, 10_000L, 9_000L)
        val state = stateWithBooks(10.0, 10_000L, 9_000L)
        val rowId = pendingSplice()
        val lock = field(state, "booksLock")
        val started = CountDownLatch(1)
        val failure = AtomicReference<Throwable?>()
        val worker = Thread {
            started.countDown()
            try {
                assertTrue(processSync(state, sync(8.0)))
            } catch (t: Throwable) {
                failure.set(t)
            }
        }
            .apply { isDaemon = true }
        try {
            synchronized(lock) {
                worker.start()
                assertTrue(started.await(5, TimeUnit.SECONDS))
                val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
                fun waitingForBooks(): Boolean =
                    worker.state == Thread.State.BLOCKED &&
                        worker.stackTrace.firstOrNull()?.let {
                            it.className == AppState::class.java.name &&
                                it.methodName.endsWith("processSignedSyncMessage")
                        } == true
                while (worker.isAlive && !waitingForBooks() && System.nanoTime() < deadline) {
                    Thread.yield()
                }
                assertTrue(
                    "SYNC must wait for the same lock as splice accounting",
                    waitingForBooks(),
                )
                assertBooks(10.0, 10_000L, 0)
                assertEquals("COMPLETED", completeSplice(state, rowId))
                assertBooks(9.0, 9_000L, 0)
            }
        } finally {
            worker.join(5_000)
        }
        assertFalse(worker.isAlive)
        failure.get()?.let { throw AssertionError("SYNC worker failed", it) }
        assertBooks(8.0, 8_000L, 1)
        assertEquals(8.0, state.stableChannel.value.expectedUSD.amount, 0.0)
        state.saveChannelToDB()
        state.onForegroundResume()
        assertBooks(8.0, 8_000L, 1)
        assertEquals(
            8f,
            context
                .getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                .getFloat("cached_expected_usd", -1f),
            0f,
        )
    }

    @Test
    fun failedSpliceAccountingLeavesTheOperationPendingAndRetriesExactlyOnce() {
        save(10.0, 10_000L, 9_000L)
        val state = stateWithBooks(10.0, 10_000L, 9_000L)
        val rowId = pendingSplice()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_splice BEFORE UPDATE ON channels BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        assertEquals("DEFERRED", completeSplice(state, rowId))
        assertBooks(10.0, 10_000L, 0)
        assertTrue(db.hasPendingSpliceFor(txid))
        db.writableDatabase.execSQL("DROP TRIGGER refuse_splice")
        assertEquals("COMPLETED", completeSplice(state, rowId))
        assertEquals("COMPLETED", completeSplice(state, rowId))
        assertBooks(9.0, 9_000L, 0)
    }

    private fun pendingSplice(): Long =
        db.recordPayment(
            "splice-payment",
            "splice_out",
            "sent",
            1_000_000L,
            status = "pending",
            txid = txid,
        )

    @Suppress("UNCHECKED_CAST")
    private fun stateWithBooks(expected: Double, backing: Long, receiver: Long): AppState =
        AppState(context).also { state ->
            AppState::class
                .java
                .getDeclaredField("databaseService")
                .apply { isAccessible = true }
                .set(state, db)
            (field(state, "_stableChannel") as MutableStateFlow<StableChannel>).value =
                StableChannel(
                    channelId = channelId,
                    userChannelId = "7",
                    expectedUSD = USD(expected),
                    backingSats = backing,
                    stableReceiverBTC = Bitcoin(receiver),
                    latestPrice = price,
                )
            state.priceService.seedPrice(price)
            (field(state.priceService, "_lastUpdate") as MutableStateFlow<Date>).value = Date()
        }

    private fun field(target: Any, name: String): Any =
        target.javaClass.getDeclaredField(name).apply { isAccessible = true }.get(target)!!

    private fun processSync(state: AppState, message: TradeControlMessage): Boolean =
        AppState::class
            .java
            .getDeclaredMethod(
                "processSignedSyncMessage",
                TradeControlMessage::class.java,
                String::class.java,
                Long::class.javaPrimitiveType,
            )
            .apply { isAccessible = true }
            .invoke(state, message, "sync-payment", TradeProtocol.RESULT_CONTROL_AMOUNT_MSAT)
            as Boolean

    private fun completeSplice(state: AppState, rowId: Long): String =
        AppState::class
            .java
            .getDeclaredMethod(
                "completeConfirmedSplice",
                String::class.java,
                Long::class.javaPrimitiveType,
                Long::class.javaObjectType,
            )
            .apply { isAccessible = true }
            // A stale monitor generation avoids scheduling UI dismissal work in this test.
            .invoke(state, txid, -1L, rowId)!!
            .toString()
}
