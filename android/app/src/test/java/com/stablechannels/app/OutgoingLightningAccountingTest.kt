package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.LightningPaymentRecovery
import com.stablechannels.app.services.LightningPaymentResolution
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.services.PaymentFailureRecorder
import com.stablechannels.app.util.Constants
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OutgoingLightningAccountingTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val price = 100_000.0

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
        db = DatabaseService(context)
        db.saveChannel("channel", "7", 100.0, 100_000L, null, 110_000L, price)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
    }

    private fun nodeService() = NodeService(context).also { service ->
        service.channelSpendGuard = { _, _ -> check(!db.hasPendingChannelSend()) }
        service.channelPaymentRecorder = { id, type, amount, _ ->
            db.recordPendingLightningPayment(id, type, amount, price)
        }
    }

    private fun reconcile(id: String, liveSats: Long) = db.reconcileOutgoingBacking(
        "channel", "7", null, liveSats, price, price, paymentId = id
    )

    @Test
    fun bolt11SuccessMustBeAccountedBeforeAnotherSendAndLateReplayCannotChargeFailedSend() {
        delayedSuccessThenFailedSend("lightning", immediateSuccess = true)
    }

    @Test
    fun bolt12SuccessMustBeAccountedBeforeAnotherSendAndLateReplayCannotChargeFailedSend() {
        delayedSuccessThenFailedSend("bolt12", immediateSuccess = false)
    }

    private fun delayedSuccessThenFailedSend(type: String, immediateSuccess: Boolean) {
        val node = nodeService()
        var liveSats = 110_000L
        if (immediateSuccess) {
            node.channelPaymentRecorder = { id, paymentType, amount, _ ->
                db.recordPendingLightningPayment(id, paymentType, amount, price)
                // LDK already returned SUCCEEDED while the success event is still queued.
                db.updatePaymentStatus(id, "completed")
            }
        }
        node.sendTrackedLightningPayment(type, 5_000_000L, price) {
            liveSats -= 5_000L
            "first"
        }
        assertTrue(db.hasPendingChannelSend())
        if (!immediateSuccess) LightningPaymentRecovery.recordSuccess(db, "first", 0L)
        assertEquals("completed", db.getRecentPayments().single().status)

        var secondSubmitted = false
        assertThrows(IllegalStateException::class.java) {
            node.sendTrackedLightningPayment(type, 10_000_000L, price) {
                secondSubmitted = true
                liveSats -= 10_000L
                "second"
            }
        }
        assertFalse(secondSubmitted)
        assertEquals(105_000L, liveSats)

        synchronized(node.channelOperationLock) { reconcile("first", liveSats) }
        assertTrue(db.isLightningAccountingComplete("first"))
        assertFalse(db.hasPendingChannelSend())
        // The delayed second attempt is now allowed. Its HTLC temporarily consumes $5 backing.
        node.channelPaymentRecorder = { id, paymentType, amount, _ ->
            db.recordPendingLightningPayment(id, paymentType, amount, price)
        }
        node.sendTrackedLightningPayment(type, 10_000_000L, price) {
            liveSats -= 10_000L
            "second"
        }
        assertEquals(95_000L, liveSats)
        // Redelivery of the first success must not account the second payment's pending HTLC.
        assertNull(reconcile("first", liveSats))
        PaymentFailureRecorder.record(db, "second", "RETRIES_EXHAUSTED") { null }
        liveSats += 10_000L
        assertFalse(db.hasPendingChannelSend())
        assertEquals(105_000L, liveSats)
        assertEquals(100.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(100_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun backgroundSuccessKeepsAccountingBlockedAcrossRestartUntilBalanceCommit() {
        db.recordPendingLightningPayment("first", "bolt12", 15_000_000L, price)
        LightningPaymentRecovery.reconcilePending(db) { LightningPaymentResolution(true, 123L) }
        assertTrue(db.getPendingOutgoingLightningPaymentIds().isEmpty())
        db.close()
        db = DatabaseService(context)
        assertTrue(db.hasPendingChannelSend())
        assertEquals(listOf("first"), db.getUnaccountedOutgoingLightningPaymentIds())
        assertEquals(95.0, reconcile("first", 95_000L)!!.newExpectedUSD, 0.0)
        db.close()
        db = DatabaseService(context)
        assertFalse(db.hasPendingChannelSend())
        assertTrue(db.getUnaccountedOutgoingLightningPaymentIds().isEmpty())
        assertNull(reconcile("first", 80_000L))
        assertEquals(95.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun accountingFailureRollsBackBothBalanceAndCompletionMarker() {
        db.recordPendingLightningPayment("first", "lightning", 15_000_000L, price)
        LightningPaymentRecovery.recordSuccess(db, "first", 0L)
        db.writableDatabase.execSQL("""
            CREATE TRIGGER fail_accounting BEFORE UPDATE OF completed ON outgoing_lightning_accounting
            WHEN NEW.completed = 1 BEGIN SELECT RAISE(ABORT, 'injected accounting failure'); END
        """)
        assertThrows(Exception::class.java) { reconcile("first", 95_000L) }
        assertEquals(100.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(100_000L, db.loadChannel("7")!!.backingSats)
        assertFalse(db.isLightningAccountingComplete("first"))
        assertTrue(db.hasPendingChannelSend())
        db.writableDatabase.execSQL("DROP TRIGGER fail_accounting")
        assertEquals(95.0, reconcile("first", 95_000L)!!.newExpectedUSD, 0.0)
        assertFalse(db.hasPendingChannelSend())
    }

    @Test
    fun missingPriceOrChannelDoesNotReleaseAccountingBarrier() {
        db.recordPendingLightningPayment("first", "lightning", 15_000_000L, price)
        for (badPrice in listOf(0.0, Double.NaN, Double.POSITIVE_INFINITY)) {
            assertThrows(IllegalStateException::class.java) {
                db.reconcileOutgoingBacking("channel", "7", null, 95_000L, price, badPrice, "first")
            }
            assertTrue(db.hasPendingChannelSend())
        }
        assertThrows(IllegalStateException::class.java) {
            db.reconcileOutgoingBacking("missing", "missing", null, 95_000L, price, price, "first")
        }
        assertFalse(db.isLightningAccountingComplete("first"))
        assertTrue(db.hasPendingChannelSend())
    }

    @Test
    fun legacyPendingRowsAcquireAccountingMarkerOnTerminalRecovery() {
        db.recordPayment("legacy", "bolt12", "sent", 15_000_000L, status = "pending")
        LightningPaymentRecovery.reconcilePending(db) { LightningPaymentResolution(true) }
        assertEquals(listOf("legacy"), db.getUnaccountedOutgoingLightningPaymentIds())
        assertTrue(db.hasPendingChannelSend())
        reconcile("legacy", 95_000L)
        assertFalse(db.hasPendingChannelSend())
    }

    @Test
    fun operationLockCoversSubmissionThroughHistoryRegistration() {
        val node = nodeService()
        val recording = CountDownLatch(1)
        val release = CountDownLatch(1)
        val secondStarted = CountDownLatch(1)
        val executor = Executors.newFixedThreadPool(2)
        node.channelPaymentRecorder = { id, type, amount, _ ->
            assertTrue(Thread.holdsLock(node.channelOperationLock))
            recording.countDown()
            check(release.await(5, TimeUnit.SECONDS))
            db.recordPendingLightningPayment(id, type, amount, price)
            db.updatePaymentStatus(id, "completed")
        }
        try {
            val first = executor.submit<String> {
                node.sendTrackedLightningPayment("lightning", 5_000_000L, price) { "first" }
            }
            assertTrue(recording.await(5, TimeUnit.SECONDS))
            val second = executor.submit<Boolean> {
                secondStarted.countDown()
                try {
                    node.sendTrackedLightningPayment("bolt12", 10_000_000L, price) { "second" }
                    false
                } catch (_: IllegalStateException) {
                    true
                }
            }
            assertTrue(secondStarted.await(5, TimeUnit.SECONDS))
            release.countDown()
            assertEquals("first", first.get(5, TimeUnit.SECONDS))
            assertTrue(second.get(5, TimeUnit.SECONDS))
            assertEquals(1, db.getRecentPayments().size)
        } finally {
            release.countDown()
            executor.shutdownNow()
        }
    }
}
