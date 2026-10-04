package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import java.io.File
import java.util.Date
import java.util.concurrent.atomic.AtomicLong
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
class SpliceTrackingRegressionTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val txid = "cd".repeat(32)
    private val channelId = "ab".repeat(32)

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        db = DatabaseService(context)
        db.saveChannel(channelId, "7", 10.0, 10_000, null, 9_000, 100_000.0)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    @Test
    fun unrelatedTransactionDoesNotMatchFailedOrUnassignedSpliceIn() {
        for (status in listOf("pending", "failed")) {
            db.recordPayment(null, "splice_in", "received", 1_000_000, status = status)
            assertFalse(db.hasPendingSpliceFor(txid))
            assertFalse(db.completeSplice(txid))
        }
    }

    @Test
    fun failedExactTransactionDoesNotPassPendingCheck() {
        db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "failed", txid = txid)
        assertFalse(db.hasPendingSpliceFor(txid))
        assertFalse(db.completeSplice(txid))
    }

    @Test
    fun unrelatedConfirmationCannotClaimANewerPendingOperationOrShowSuccess() {
        val rowId = db.recordPayment(null, "splice_in", "received", 1_000_000, status = "pending")
        val state = state()
        assertEquals("DEFERRED", complete(state, null))
        assertEquals("pending", db.getRecentPayments().single { it.id == rowId }.status)
        assertNull(db.getRecentPayments().single { it.id == rowId }.txid)
        assertNotEquals("Move confirmed", state.statusMessage.value)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun failedOperationCannotShowSuccess() {
        val rowId =
            db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "failed", txid = txid)
        val state = state()
        assertEquals("DEFERRED", complete(state, rowId))
        assertNotEquals("Move confirmed", state.statusMessage.value)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun failedHistoryWriteKeepsCompletionRetryable() {
        val rowId =
            db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "pending", txid = txid)
        val state = state()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_completion BEFORE UPDATE OF status ON payments BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        assertEquals("DEFERRED", complete(state, rowId))
        assertTrue(db.hasPendingSpliceFor(txid))
        assertNotEquals("Move confirmed", state.statusMessage.value)
        db.writableDatabase.execSQL("DROP TRIGGER reject_completion")
        assertEquals("COMPLETED", complete(state, rowId))
        assertEquals("Move confirmed", state.statusMessage.value)
        assertEquals("COMPLETED", complete(state, rowId))
        assertEquals(9.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun capturedRowCannotCompleteAnotherRowsTransaction() {
        val first =
            db.recordPayment(
                null,
                "splice_in",
                "received",
                1_000_000,
                status = "pending",
                txid = "other",
            )
        db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "pending", txid = txid)
        assertEquals("DEFERRED", complete(state(), first))
        assertTrue(db.hasPendingSpliceFor(txid))
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Suppress("UNCHECKED_CAST")
    private fun state(): AppState =
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
                    expectedUSD = USD(10.0),
                    backingSats = 10_000,
                    stableReceiverBTC = Bitcoin(9_000),
                    latestPrice = 100_000.0,
                )
            state.priceService.seedPrice(100_000.0)
            (field(state.priceService, "_lastUpdate") as MutableStateFlow<Date>).value = Date()
        }

    private fun field(target: Any, name: String): Any =
        target.javaClass.getDeclaredField(name).apply { isAccessible = true }.get(target)!!

    /**
     * Runs against the live generation, so a completion that reaches the terminal step really shows
     * "Move confirmed".
     */
    private fun complete(state: AppState, rowId: Long?): String =
        AppState::class
            .java
            .getDeclaredMethod(
                "completeConfirmedSplice",
                String::class.java,
                Long::class.javaPrimitiveType,
                Long::class.javaObjectType,
            )
            .apply { isAccessible = true }
            .invoke(state, txid, (field(state, "spliceGeneration") as AtomicLong).get(), rowId)!!
            .toString()
}
