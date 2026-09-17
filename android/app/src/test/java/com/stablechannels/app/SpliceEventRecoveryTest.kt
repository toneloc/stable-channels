package com.stablechannels.app

import android.app.Notification
import android.content.Context
import android.content.Intent
import android.database.sqlite.SQLiteException
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.PendingSplice
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.LdkBackgroundService
import com.stablechannels.app.services.SpliceBroadcastChecker
import com.stablechannels.app.services.SpliceEventRecorder
import com.stablechannels.app.util.Constants
import java.io.File
import java.util.Date
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.Event
import org.lightningdevkit.ldknode.OutPoint
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], shadows = [SystemCleanerShadow::class])
class SpliceEventRecoveryTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val channel = "ab".repeat(32)
    private val txid = "cd".repeat(32)
    private val oldTxid = "ef".repeat(32)
    private val funding = OutPoint(txid, 2u)
    private val negotiated
        get() = Event.SpliceNegotiated(channel, "7", "peer", funding)

    private val ready
        get() = Event.ChannelReady(channel, "7", "peer", funding)

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        db = DatabaseService(context)
        db.saveChannel(channel, "7", 10.0, 10_000, null, 10_000, 100_000.0)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    private fun pending(
        type: String = "splice_out",
        user: String = "7",
        channelId: String = channel,
    ): Long =
        db.recordPendingSplice(
            type,
            1_000_000,
            1.0,
            100_000.0,
            user,
            channelId,
            previousFundingTxid = oldTxid,
        )

    private fun record(event: Event): Boolean = SpliceEventRecorder.record(db, event) { null }

    private fun payment(id: Long) = db.getRecentPayments(100).single { it.id == id }

    private fun reopen() {
        db.close()
        db = DatabaseService(context)
    }

    private fun appState(): AppState = AppState(context).also { set(it, "databaseService", db) }

    private fun set(target: Any, name: String, value: Any?) =
        target.javaClass.getDeclaredField(name).apply { isAccessible = true }.set(target, value)

    private fun read(target: Any, name: String): Any? =
        target.javaClass.getDeclaredField(name).apply { isAccessible = true }.get(target)

    private fun handle(state: AppState, event: Event) {
        AppState::class
            .java
            .getDeclaredMethod("handleEvent", Event::class.java)
            .apply { isAccessible = true }
            .invoke(state, event)
    }

    private fun tick(state: AppState) {
        AppState::class
            .java
            .getDeclaredMethod("runStabilityCheck")
            .apply { isAccessible = true }
            .invoke(state)
    }

    private fun resume(state: AppState) {
        AppState::class
            .java
            .getDeclaredMethod("resumePendingSpliceConfirmation")
            .apply { isAccessible = true }
            .invoke(state)
    }

    @Suppress("UNCHECKED_CAST")
    private fun setChannel(state: AppState, userChannelId: String) {
        (read(state, "_stableChannel") as MutableStateFlow<StableChannel>).value =
            StableChannel(channelId = channel, userChannelId = userChannelId)
    }

    /** Every esplora URL, including the hard-coded public fallbacks, is answered by the mock. */
    private fun routeEsploraTo(
        state: AppState,
        server: MockWebServer,
        response: () -> MockResponse,
    ) {
        server.dispatcher =
            object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse = response()
            }
        val client =
            OkHttpClient.Builder()
                .addInterceptor { chain ->
                    val url =
                        chain
                            .request()
                            .url
                            .newBuilder()
                            .scheme("http")
                            .host(server.hostName)
                            .port(server.port)
                            .build()
                    chain.proceed(chain.request().newBuilder().url(url).build())
                }
                .build()
        set(state, "httpClient", client)
        set(state, "spliceBroadcastChecker", SpliceBroadcastChecker(client, sleep = {}))
        set(state, "chainUrl", server.url("/").toString())
    }

    private fun expire(id: Long) {
        db.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - 3600 WHERE id = ?",
            arrayOf(id),
        )
    }

    /**
     * Books worth $10 backed by 10,000 sats, with 9,000 sats left in the channel after a 1,000-sat
     * move out.
     */
    @Suppress("UNCHECKED_CAST")
    private fun setBooks(state: AppState) {
        db.saveChannel(channel, "7", 10.0, 10_000, null, 9_000, 100_000.0)
        (read(state, "_stableChannel") as MutableStateFlow<StableChannel>).value =
            StableChannel(
                channelId = channel,
                userChannelId = "7",
                expectedUSD = USD(10.0),
                backingSats = 10_000,
                stableReceiverBTC = Bitcoin(9_000),
                latestPrice = 100_000.0,
            )
        state.priceService.seedPrice(100_000.0)
        (read(state.priceService, "_lastUpdate") as MutableStateFlow<Date>).value = Date()
    }

    private fun generation(state: AppState) = (read(state, "spliceGeneration") as AtomicLong).get()

    /**
     * A move out is a channel spend: the guard needs a running node listing the channel at par and
     * a fresh price.
     */
    @Suppress("UNCHECKED_CAST")
    private fun installNode(state: AppState, receiver: Long = 10_000): TestNode {
        val node =
            TestNode().also { it.channels = listOf(channel(cid = channel, receiver = receiver)) }
        set(state.nodeService, "node", node)
        (read(state.nodeService, "_isRunning") as MutableStateFlow<Boolean>).value = true
        state.priceService.seedPrice(100_000.0)
        (read(state.priceService, "_lastUpdate") as MutableStateFlow<Date>).value = Date()
        return node
    }

    @Test
    fun negotiatedEventsSurviveBackgroundHandoffRestartAndReplayForBothDirections() {
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

    @Test
    fun readyAloneRecoversWithUnchangedChannelIdAndUsesTheEventFunding() {
        val id = pending()
        // LDK keeps the channel ID across a splice. Its funding outpoint identifies the move.
        assertTrue(SpliceEventRecorder.record(db, ready) { OutPoint("wrong-live-snapshot", 0u) })
        reopen()
        assertEquals(txid, payment(id).txid)
        assertEquals(txid, db.getPendingSpliceTxid())
        assertEquals("pending", payment(id).status)
        assertEquals(0, payment(id).confirmations)
    }

    @Test
    fun delayedNegotiationRetainsItsCapturedOperationPastThePreNegotiationTimeout() {
        val id = pending()
        db.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - 3600 WHERE id = ?",
            arrayOf(id),
        )
        assertTrue(record(negotiated))
        reopen()
        assertTrue(db.hasPendingSplice())
        assertEquals(txid, payment(id).txid)
    }

    @Test
    fun journalFailureCannotAcknowledgeOrPartiallyAssignAndRetryWorks() {
        val id = pending()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_event BEFORE INSERT ON splice_events BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        assertThrows(SQLiteException::class.java) { record(negotiated) }
        assertNull(payment(id).txid)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_event")
        assertTrue(record(negotiated))
        assertEquals(txid, payment(id).txid)
    }

    @Test
    fun assignmentFailureRollsBackJournalAndRetriesAfterReopen() {
        val id = pending()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_assignment BEFORE UPDATE OF txid ON payments BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        assertThrows(SQLiteException::class.java) { record(negotiated) }
        assertEquals(
            0,
            db.readableDatabase.rawQuery("SELECT COUNT(*) FROM splice_events", null).use {
                it.moveToFirst()
                it.getInt(0)
            },
        )
        assertNull(payment(id).txid)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_assignment")
        reopen()
        assertTrue(record(negotiated))
        assertEquals(txid, payment(id).txid)
    }

    @Test
    fun unrelatedChannelAndAmbiguousRowsRemainUnassignedAfterReplayAndNewOperations() {
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

    @Test
    fun failedOrCompletedOldEventsCannotClaimANewOperation() {
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

    @Test
    fun initialChannelReadyCannotClaimAPendingSplice() {
        val id = pending()
        assertTrue(record(ready.copy(fundingTxo = OutPoint(oldTxid, 0u))))
        assertNull(payment(id).txid)
        assertTrue(record(ready))
        assertEquals(txid, payment(id).txid)
    }

    @Test
    fun legacyReadyWithoutFundingIsSavedThenResolvedAgainstTheSameChannel() {
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

    @Test
    fun backgroundAcknowledgesBothSpliceEventsOnlyAfterTheDatabaseCommit() {
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

    @Test
    fun backgroundDatabaseFailureLeavesTheEventUnacknowledgedAndRetryable() {
        pending()
        val background = Robolectric.buildService(StabilityProcessingService::class.java).get()
        var acknowledgements = 0
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_background_event BEFORE INSERT ON splice_events BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
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

    @Test
    fun foregroundResumeReconstructsTheExactOperationAndDuplicateResumeKeepsItsMonitor() {
        val id = pending()
        record(negotiated)
        reopen()
        val state = AppState(context)
        AppState::class
            .java
            .getDeclaredField("databaseService")
            .apply { isAccessible = true }
            .set(state, db)
        val server = MockWebServer()
        server.start()
        routeEsploraTo(state, server) { MockResponse().setBody("{\"confirmed\":false}") }
        val resume =
            AppState::class.java.getDeclaredMethod("resumePendingSpliceConfirmation").apply {
                isAccessible = true
            }
        fun field(name: String): Any? =
            AppState::class.java.getDeclaredField(name).apply { isAccessible = true }.get(state)
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

    @Test
    fun legacyReadyForOriginalFundingDoesNotPreventAnAbandonedOperationTimingOut() {
        val id = pending()
        record(ready.copy(fundingTxo = null))
        SpliceEventRecorder.recoverReadyEvents(db) { _, _ -> OutPoint(oldTxid, 0u) }
        assertNull(payment(id).txid)
        db.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - 3600 WHERE id = ?",
            arrayOf(id),
        )
        assertFalse(db.hasPendingSplice())
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun databaseUpgradeAndLegacyNegotiationKeepExistingHistoryAndBooks() {
        val id = db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "pending")
        db.writableDatabase.execSQL("DROP TABLE splice_events")
        db.writableDatabase.execSQL("DROP TABLE splice_operations")
        reopen()
        record(negotiated)
        assertEquals(txid, payment(id).txid)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun legacyReadyReplayCannotRecaptureANewerOperationFromALiveSnapshot() {
        pending()
        val legacy = ready.copy(fundingTxo = null)
        assertTrue(SpliceEventRecorder.record(db, legacy) { funding })
        assertTrue(db.completeSplice(txid))
        val later = pending()
        reopen()
        assertTrue(SpliceEventRecorder.record(db, legacy) { OutPoint("new-live-funding", 0u) })
        assertNull(payment(later).txid)
    }

    @Test
    fun negotiationForTheExistingFundingCannotClaimANewSplice() {
        val id = pending()
        record(negotiated.copy(newFundingTxo = OutPoint(oldTxid, 0u)))
        assertNull(payment(id).txid)
        record(negotiated)
        assertEquals(txid, payment(id).txid)
    }

    @Test
    fun restartKeepsNewSplicesBlockedWhileTheTransactionIdIsStillUnknown() {
        val id = pending()
        reopen()
        val state = AppState(context)
        AppState::class
            .java
            .getDeclaredField("databaseService")
            .apply { isAccessible = true }
            .set(state, db)
        AppState::class
            .java
            .getDeclaredMethod("resumePendingSpliceConfirmation")
            .apply { isAccessible = true }
            .invoke(state)
        assertTrue(state.isSpliceInFlight)
        assertNull(payment(id).txid)
        assertEquals("pending", payment(id).status)
    }

    @Test
    fun newChannelWithNoSpliceDoesNotCreateOrCompleteAPayment() {
        assertTrue(record(ready))
        reopen()
        assertFalse(db.hasPendingSplice())
        assertTrue(db.getRecentPayments().isEmpty())
    }

    @Test
    fun duplicateTxidRowsAreNeverAmbiguouslyCompleted() {
        repeat(2) {
            db.recordPayment(null, "splice_out", "sent", 1_000_000, status = "pending", txid = txid)
        }
        assertFalse(db.hasPendingSpliceFor(txid))
        assertFalse(db.completeSplice(txid))
        assertTrue(db.getRecentPayments().all { it.status == "pending" })
    }

    @Test
    fun operationIdentityFailureRollsBackThePaymentToo() {
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_operation BEFORE INSERT ON splice_operations BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        assertThrows(SQLiteException::class.java) { pending() }
        assertTrue(db.getRecentPayments().isEmpty())
    }

    @Test
    fun aFirstChannelReadyCreatesTheChannelRowAndLaterOnesPreserveBacking() {
        val state = appState()
        setChannel(state, "9")
        val firstReady = Event.ChannelReady(channel, "9", "peer", OutPoint(oldTxid, 0u))
        handle(state, firstReady)
        assertEquals(0L, db.loadChannel("9")!!.backingSats)
        // A stability credit committed by the background process must survive the next metadata
        // save.
        db.saveChannel(channel, "9", 5.0, 5_000, null)
        handle(state, firstReady)
        assertEquals(5_000L, db.loadChannel("9")!!.backingSats)
    }

    @Test
    fun aFailureDeliveredAfterRestartFailsTheNegotiatedOperationEsploraNeverSaw() {
        val id = pending()
        record(negotiated)
        reopen()
        val state = appState()
        val server = MockWebServer().also { it.start() }
        try {
            routeEsploraTo(state, server) { MockResponse().setResponseCode(404) }
            handle(state, Event.SpliceNegotiationFailed(channel, "7", "peer"))
            assertEquals("failed", payment(id).status)
            assertFalse(state.isSpliceInFlight)
            assertFalse(db.hasPendingSplice())
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun aStaleFailureReplayForABroadcastTransactionKeepsTheOperationPending() {
        val id = pending()
        record(negotiated)
        reopen()
        val state = appState()
        val server = MockWebServer().also { it.start() }
        try {
            routeEsploraTo(state, server) { MockResponse().setBody("{\"confirmed\":false}") }
            handle(state, Event.SpliceNegotiationFailed(channel, "7", "peer"))
            assertEquals("pending", payment(id).status)
            assertTrue(db.hasPendingSplice())
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun anExpiredPreNegotiationOperationReleasesTheRestartLockForTheNextMove() {
        val id = pending()
        val state = appState()
        setChannel(state, "7")
        installNode(state)
        resume(state)
        assertTrue(state.isSpliceInFlight)
        assertThrows(IllegalStateException::class.java) {
            state.beginSpliceOut(1_000, "addr", 100_000.0)
        }
        expire(id)
        state.beginSpliceOut(1_000, "addr", 100_000.0)
        assertEquals("failed", payment(id).status)
        assertTrue(state.isSpliceInFlight)
        assertEquals(2, db.getRecentPayments(100).size)
    }

    @Test
    fun resumeReleasesALockWhoseOperationExpiredWhileTheAppRan() {
        val id = pending()
        val state = appState()
        resume(state)
        assertTrue(state.isSpliceInFlight)
        expire(id)
        resume(state)
        assertFalse(state.isSpliceInFlight)
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun backgroundLeavesASpliceFailureForTheForegroundOnlyWhileAnOperationIsPending() {
        val background = Robolectric.buildService(StabilityProcessingService::class.java).get()
        var acknowledgements = 0
        background.deferSpliceFailure { acknowledgements++ }
        assertEquals(1, acknowledgements)
        pending()
        assertThrows(Exception::class.java) { background.deferSpliceFailure { acknowledgements++ } }
        assertEquals(1, acknowledgements)
    }

    @Test
    fun anUnresolvedLegacyReadyDoesNotExemptAnAbandonedOperationFromTheTimeout() {
        val id = pending()
        assertTrue(record(ready.copy(fundingTxo = null)))
        SpliceEventRecorder.recoverReadyEvents(db) { _, _ -> null }
        expire(id)
        assertFalse(db.hasPendingSplice())
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun aNegotiatedTxidTheAssignmentRefusesCannotExemptTheOperationFromTheTimeout() {
        val id = pending()
        // A row the assignment never reclaims already carries the negotiated txid.
        db.recordPayment(null, "onchain", "sent", 1_000_000, txid = txid)
        assertTrue(record(negotiated))
        assertNull(payment(id).txid)
        expire(id)
        assertFalse(db.hasPendingSplice())
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun theStabilityTickReleasesALockWhoseOperationExpiredWithoutAnyUserAction() {
        val id = pending()
        val state = appState()
        setChannel(state, "7")
        resume(state)
        assertTrue(state.isSpliceInFlight)
        expire(id)
        // A claimed send makes the tick bail out right after the release, so the release must come
        // first.
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        tick(state)
        assertFalse(state.isSpliceInFlight)
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun aFailingStaleLockCheckOnTheTickNeitherCrashesNorReleasesTheLock() {
        val id = pending()
        val state = appState()
        setChannel(state, "7")
        resume(state)
        expire(id)
        db.writableDatabase.execSQL(
            "CREATE TRIGGER refuse_expiry BEFORE UPDATE OF status ON payments BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        tick(state)
        assertTrue(state.isSpliceInFlight)
        assertEquals("pending", payment(id).status)
        db.writableDatabase.execSQL("DROP TRIGGER refuse_expiry")
        tick(state)
        assertFalse(state.isSpliceInFlight)
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun aMoveOutCompletesEndToEndFromBeginThroughNegotiationAndConfirmation() {
        val state = appState()
        setBooks(state)
        val node = installNode(state)
        val server = MockWebServer().also { it.start() }
        var job: Job? = null
        try {
            routeEsploraTo(state, server) { MockResponse().setBody("{\"confirmed\":false}") }
            state.beginSpliceOut(1_000, "addr", 100_000.0)
            node.channels =
                listOf(channel(cid = channel, receiver = 9_000)) // the 1,000-sat move left
            val rowId = (read(state, "pendingSplice") as PendingSplice).paymentRowId
            handle(state, negotiated)
            job = read(state, "spliceConfirmationJob") as Job
            assertNotNull(server.takeRequest(5, TimeUnit.SECONDS))
            assertEquals(txid, payment(rowId).txid)
            assertEquals(txid, read(state, "spliceTxid"))
            // The monitor cannot sync a wallet here, so confirm through the same completion step it
            // runs.
            runBlocking { job.cancelAndJoin() }
            val completion =
                AppState::class
                    .java
                    .getDeclaredMethod(
                        "completeConfirmedSplice",
                        String::class.java,
                        Long::class.javaPrimitiveType,
                        Long::class.javaObjectType,
                    )
                    .apply { isAccessible = true }
                    .invoke(state, txid, generation(state), rowId)!!
                    .toString()
            assertEquals("COMPLETED", completion)
            assertEquals("completed", payment(rowId).status)
            assertEquals(txid, payment(rowId).txid)
            assertFalse(state.isSpliceInFlight)
            assertNull(read(state, "pendingSplice"))
            assertEquals("Move confirmed", state.statusMessage.value)
            assertEquals(9.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        } finally {
            runBlocking { job?.cancelAndJoin() }
            server.shutdown()
        }
    }

    @Test
    fun aReplayedNegotiationKeepsTheLiveOperationsMonitorAndGeneration() {
        val state = appState()
        setChannel(state, "7")
        installNode(state)
        val server = MockWebServer().also { it.start() }
        var job: Job? = null
        try {
            routeEsploraTo(state, server) { MockResponse().setBody("{\"confirmed\":false}") }
            state.beginSpliceOut(1_000, "addr", 100_000.0)
            val rowId = (read(state, "pendingSplice") as PendingSplice).paymentRowId
            handle(state, negotiated)
            job = read(state, "spliceConfirmationJob") as Job
            assertNotNull(server.takeRequest(5, TimeUnit.SECONDS))
            val generation = generation(state)
            handle(state, negotiated)
            assertSame(job, read(state, "spliceConfirmationJob"))
            assertEquals(generation, generation(state))
            assertEquals(rowId, (read(state, "pendingSplice") as PendingSplice).paymentRowId)
            assertEquals(txid, read(state, "spliceTxid"))
            assertTrue(state.isSpliceInFlight)
        } finally {
            runBlocking { job?.cancelAndJoin() }
            server.shutdown()
        }
    }

    @Test
    fun backgroundingKeepsTheNodeAliveOnlyWhileASpliceIsStillNegotiating() {
        val state = appState()
        setChannel(state, "7")
        installNode(state)
        val app = RuntimeEnvironment.getApplication()
        fun keepAliveReason(): String? {
            state.stopNodeForBackground()
            val started = shadowOf(app).nextStartedService
            (read(state, "backgroundStopJob") as? Job)?.cancel()
            if (started?.component?.className != LdkBackgroundService::class.java.name) return null
            return started.getStringExtra(LdkBackgroundService.EXTRA_REASON)
        }
        assertNull(keepAliveReason())
        state.beginSpliceOut(1_000, "addr", 100_000.0)
        assertEquals(LdkBackgroundService.REASON_SPLICE, keepAliveReason())
        // Alongside a payment wait the move still wins: it is the hold that dies with the node.
        state.isWaitingForPayment = true
        assertEquals(LdkBackgroundService.REASON_SPLICE, keepAliveReason())
        state.isWaitingForPayment = false
        // Once the transaction is negotiated, confirmation survives a node stop; nothing to hold.
        state.spliceTxid = txid
        assertNull(keepAliveReason())
        state.spliceTxid = null
        state.cancelPendingSpliceStart()
        assertNull(keepAliveReason())
        // A lock restored after a restart guards a negotiation that died with the old process.
        pending()
        resume(state)
        assertTrue(state.isSpliceInFlight)
        assertNull(keepAliveReason())
        // A payment wait still keeps its own notification text.
        state.isWaitingForPayment = true
        assertEquals(LdkBackgroundService.REASON_PAYMENT, keepAliveReason())
    }

    @Test
    fun theKeepAliveNotificationNamesTheSpliceWhenThatIsWhatHoldsTheNode() {
        val app = RuntimeEnvironment.getApplication()
        fun notificationText(reason: String?): String {
            val intent = Intent(app, LdkBackgroundService::class.java)
            if (reason != null) intent.putExtra(LdkBackgroundService.EXTRA_REASON, reason)
            val controller =
                Robolectric.buildService(LdkBackgroundService::class.java, intent)
                    .create()
                    .startCommand(0, 1)
            val notification = shadowOf(controller.get()).lastForegroundNotification
            return notification.extras.getCharSequence(Notification.EXTRA_TEXT).toString()
        }
        assertEquals(
            "Stable Channels is completing your on-chain move...",
            notificationText(LdkBackgroundService.REASON_SPLICE),
        )
        assertEquals(
            "Stable Channels is waiting for your payment to complete...",
            notificationText(LdkBackgroundService.REASON_PAYMENT),
        )
        assertEquals(
            "Stable Channels is waiting for your payment to complete...",
            notificationText(null),
        )
    }
}
