package com.stablechannels.app

import android.content.Context
import android.database.sqlite.SQLiteException
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.models.PendingSplice
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.SpliceEventRecorder
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.robolectric.Robolectric
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.Event
import org.lightningdevkit.ldknode.OutPoint
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SpliceEventRecoveryTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val channel = "ab".repeat(32)
    private val txid = "cd".repeat(32)
    private val oldTxid = "ef".repeat(32)
    private val funding = OutPoint(txid, 2u)
    private val negotiated get() = Event.SpliceNegotiated(channel, "7", "peer", funding)
    private val ready get() = Event.ChannelReady(channel, "7", "peer", funding)

    @Before fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
        db = DatabaseService(context)
        db.saveChannel(channel, "7", 10.0, 10_000, null, 10_000, 100_000.0)
    }

    @After fun tearDown() {
        db.close()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
    }

    private fun pending(type: String = "splice_out", user: String = "7", channelId: String = channel): Long =
        db.recordPendingSplice(type, 1_000_000, 1.0, 100_000.0, user, channelId,
            previousFundingTxid = oldTxid)

    private fun record(event: Event): Boolean = SpliceEventRecorder.record(db, event) { null }
    private fun payment(id: Long) = db.getRecentPayments(100).single { it.id == id }
    private fun reopen() { db.close(); db = DatabaseService(context) }

    @Test fun negotiatedEventsSurviveBackgroundHandoffRestartAndReplayForBothDirections() {
        for (type in listOf("splice_out", "splice_in")) {
            val id = pending(type)
            val event = negotiated.copy(newFundingTxo = OutPoint("$txid-$type", 2u))
            // A separate helper represents the background process using the same durable DB.
            DatabaseService(context).use { background ->
                assertTrue(SpliceEventRecorder.record(background, event) { null })
            }
            reopen()
            assertTrue(db.hasPendingSplice())
            assertEquals(event.newFundingTxo.txid, db.getPendingSpliceTxid())
            assertEquals(event.newFundingTxo.txid, payment(id).txid)
            assertTrue(record(event))
            assertTrue(db.completeSplice(event.newFundingTxo.txid))
            assertTrue(record(event))
            assertEquals("completed", payment(id).status)
        }
    }

    @Test fun readyAloneRecoversWithUnchangedChannelIdAndUsesTheEventFunding() {
        val id = pending()
        // LDK keeps the channel ID across a splice. Its funding outpoint identifies the move.
        assertTrue(SpliceEventRecorder.record(db, ready) { OutPoint("wrong-live-snapshot", 0u) })
        reopen()
        assertEquals(txid, payment(id).txid)
        assertEquals(txid, db.getPendingSpliceTxid())
        assertEquals("pending", payment(id).status)
        assertEquals(0, payment(id).confirmations)
    }

    @Test fun delayedNegotiationRetainsItsCapturedOperationPastThePreNegotiationTimeout() {
        val id = pending()
        db.writableDatabase.execSQL("UPDATE payments SET created_at = created_at - 3600 WHERE id = ?", arrayOf(id))
        assertTrue(record(negotiated))
        reopen()
        assertTrue(db.hasPendingSplice())
        assertEquals(txid, payment(id).txid)
    }

    @Test fun journalFailureCannotAcknowledgeOrPartiallyAssignAndRetryWorks() {
        val id = pending()
        db.writableDatabase.execSQL("CREATE TRIGGER refuse_event BEFORE INSERT ON splice_events BEGIN SELECT RAISE(ABORT, 'test'); END")
        assertThrows(SQLiteException::class.java) { record(negotiated) }
        assertNull(payment(id).txid)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_event")
        assertTrue(record(negotiated))
        assertEquals(txid, payment(id).txid)
    }

    @Test fun assignmentFailureRollsBackJournalAndRetriesAfterReopen() {
        val id = pending()
        db.writableDatabase.execSQL("CREATE TRIGGER refuse_assignment BEFORE UPDATE OF txid ON payments BEGIN SELECT RAISE(ABORT, 'test'); END")
        assertThrows(SQLiteException::class.java) { record(negotiated) }
        assertEquals(0, db.readableDatabase.rawQuery("SELECT COUNT(*) FROM splice_events", null).use { it.moveToFirst(); it.getInt(0) })
        assertNull(payment(id).txid)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_assignment")
        reopen()
        assertTrue(record(negotiated))
        assertEquals(txid, payment(id).txid)
    }

    @Test fun unrelatedChannelAndAmbiguousRowsRemainUnassignedAfterReplayAndNewOperations() {
        val wrong = pending(user = "8", channelId = "other-channel")
        assertTrue(record(negotiated))
        val later = pending()
        assertTrue(record(negotiated))
        assertNull(payment(wrong).txid)
        assertNull(payment(later).txid)
        val another = pending()
        val ambiguous = negotiated.copy(newFundingTxo = OutPoint("ambiguous", 0u))
        assertTrue(record(ambiguous))
        assertTrue(db.failPendingSplice(another))
        reopen()
        assertTrue(record(ambiguous))
        assertNull(payment(later).txid)
    }

    @Test fun failedOrCompletedOldEventsCannotClaimANewOperation() {
        val first = pending()
        assertTrue(record(negotiated))
        assertTrue(db.completeSplice(txid))
        val second = pending()
        assertTrue(record(ready))
        assertTrue(record(negotiated))
        assertNull(payment(second).txid)
        assertEquals("completed", payment(first).status)
        assertTrue(db.failPendingSplice(second))
        assertTrue(record(negotiated.copy(newFundingTxo = OutPoint("failed-event", 0u))))
        assertEquals("failed", payment(second).status)
        assertNull(payment(second).txid)
    }

    @Test fun initialChannelReadyCannotClaimAPendingSplice() {
        val id = pending()
        assertTrue(record(ready.copy(fundingTxo = OutPoint(oldTxid, 0u))))
        assertNull(payment(id).txid)
        assertTrue(record(ready))
        assertEquals(txid, payment(id).txid)
    }

    @Test fun legacyReadyWithoutFundingIsSavedThenResolvedAgainstTheSameChannel() {
        val id = pending()
        assertTrue(record(ready.copy(fundingTxo = null)))
        reopen()
        SpliceEventRecorder.recoverReadyEvents(db) { user, chan ->
            assertEquals("7", user)
            assertEquals(channel, chan)
            funding
        }
        assertEquals(txid, payment(id).txid)
    }

    @Test fun backgroundAcknowledgesBothSpliceEventsOnlyAfterTheDatabaseCommit() {
        val id = pending()
        val background = Robolectric.buildService(StabilityProcessingService::class.java).get()
        for (event in listOf(negotiated, ready)) {
            var acknowledgements = 0
            background.persistSpliceEvent(event, { null }) {
                // A separate connection must see committed history before LDK loses the event.
                DatabaseService(context).use { reader ->
                    assertEquals(id, reader.getSplice(txid)?.id)
                }
                acknowledgements++
            }
            assertEquals(1, acknowledgements)
        }
    }

    @Test fun backgroundDatabaseFailureLeavesTheEventUnacknowledgedAndRetryable() {
        pending()
        val background = Robolectric.buildService(StabilityProcessingService::class.java).get()
        var acknowledgements = 0
        db.writableDatabase.execSQL("CREATE TRIGGER refuse_background_event BEFORE INSERT ON splice_events BEGIN SELECT RAISE(ABORT, 'test'); END")
        for (event in listOf(negotiated, ready)) {
            assertThrows(Exception::class.java) {
                background.persistSpliceEvent(event, { null }) { acknowledgements++ }
            }
        }
        assertEquals(0, acknowledgements)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_background_event")
        background.persistSpliceEvent(negotiated, { null }) { acknowledgements++ }
        assertEquals(1, acknowledgements)
    }

    @Test fun foregroundResumeReconstructsTheExactOperationAndDuplicateResumeKeepsItsMonitor() {
        val id = pending()
        record(negotiated)
        reopen()
        val state = AppState(context)
        AppState::class.java.getDeclaredField("databaseService").apply { isAccessible = true }.set(state, db)
        val server = MockWebServer()
        server.enqueue(MockResponse().setBody("{\"confirmed\":false}"))
        server.start()
        AppState::class.java.getDeclaredField("chainUrl").apply { isAccessible = true }
            .set(state, server.url("/").toString())
        val resume = AppState::class.java.getDeclaredMethod("resumePendingSpliceConfirmation")
            .apply { isAccessible = true }
        fun field(name: String): Any? = AppState::class.java.getDeclaredField(name)
            .apply { isAccessible = true }.get(state)
        var job: Job? = null
        try {
            resume.invoke(state)
            job = field("spliceConfirmationJob") as Job
            assertNotNull(server.takeRequest(5, TimeUnit.SECONDS))
            val restored = field("pendingSplice") as PendingSplice
            assertEquals(id, restored.paymentRowId)
            assertEquals(1_000L, restored.amountSats)
            assertEquals(txid, field("spliceTxid"))
            val generation = (field("spliceGeneration") as AtomicLong).get()
            resume.invoke(state)
            assertSame(job, field("spliceConfirmationJob"))
            assertEquals(generation, (field("spliceGeneration") as AtomicLong).get())
            assertEquals("pending", payment(id).status)
        } finally {
            runBlocking { job?.cancelAndJoin() }
            server.shutdown()
        }
    }

    @Test fun legacyReadyForOriginalFundingDoesNotPreventAnAbandonedOperationTimingOut() {
        val id = pending()
        record(ready.copy(fundingTxo = null))
        SpliceEventRecorder.recoverReadyEvents(db) { _, _ -> OutPoint(oldTxid, 0u) }
        assertNull(payment(id).txid)
        db.writableDatabase.execSQL("UPDATE payments SET created_at = created_at - 3600 WHERE id = ?", arrayOf(id))
        assertFalse(db.hasPendingSplice())
        assertEquals("failed", payment(id).status)
    }

    @Test fun databaseUpgradeAndLegacyNegotiationKeepExistingHistoryAndBooks() {
        val id = db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "pending")
        db.writableDatabase.execSQL("DROP TABLE splice_events")
        db.writableDatabase.execSQL("DROP TABLE splice_operations")
        reopen()
        record(negotiated)
        assertEquals(txid, payment(id).txid)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test fun legacyReadyReplayCannotRecaptureANewerOperationFromALiveSnapshot() {
        pending()
        val legacy = ready.copy(fundingTxo = null)
        assertTrue(SpliceEventRecorder.record(db, legacy) { funding })
        assertTrue(db.completeSplice(txid))
        val later = pending()
        reopen()
        assertTrue(SpliceEventRecorder.record(db, legacy) { OutPoint("new-live-funding", 0u) })
        assertNull(payment(later).txid)
    }

    @Test fun negotiationForTheExistingFundingCannotClaimANewSplice() {
        val id = pending()
        record(negotiated.copy(newFundingTxo = OutPoint(oldTxid, 0u)))
        assertNull(payment(id).txid)
        record(negotiated)
        assertEquals(txid, payment(id).txid)
    }

    @Test fun restartKeepsNewSplicesBlockedWhileTheTransactionIdIsStillUnknown() {
        val id = pending()
        reopen()
        val state = AppState(context)
        AppState::class.java.getDeclaredField("databaseService").apply { isAccessible = true }.set(state, db)
        AppState::class.java.getDeclaredMethod("resumePendingSpliceConfirmation")
            .apply { isAccessible = true }.invoke(state)
        assertTrue(state.isSpliceInFlight)
        assertNull(payment(id).txid)
        assertEquals("pending", payment(id).status)
    }

    @Test fun newChannelWithNoSpliceDoesNotCreateOrCompleteAPayment() {
        assertTrue(record(ready))
        reopen()
        assertFalse(db.hasPendingSplice())
        assertTrue(db.getRecentPayments().isEmpty())
    }

    @Test fun duplicateTxidRowsAreNeverAmbiguouslyCompleted() {
        repeat(2) { db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "pending", txid = txid) }
        assertFalse(db.hasPendingSpliceFor(txid))
        assertFalse(db.completeSplice(txid))
        assertTrue(db.getRecentPayments().all { it.status == "pending" })
    }

    @Test fun operationIdentityFailureRollsBackThePaymentToo() {
        db.writableDatabase.execSQL("CREATE TRIGGER refuse_operation BEFORE INSERT ON splice_operations BEGIN SELECT RAISE(ABORT, 'test'); END")
        assertThrows(SQLiteException::class.java) { pending() }
        assertTrue(db.getRecentPayments().isEmpty())
    }
}
