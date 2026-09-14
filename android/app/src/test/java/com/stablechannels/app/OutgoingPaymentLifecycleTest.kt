package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.*
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import java.io.File
import java.lang.reflect.InvocationTargetException
import java.util.Date

/** Real AppState event/recovery entry points and SQLite; only the LDK transport is substituted. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], shadows = [OutgoingPaymentLifecycleTest.SystemCleanerShadow::class])
class OutgoingPaymentLifecycleTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private lateinit var state: AppState
    private lateinit var node: TestNode
    private val success = Event.PaymentSuccessful("send", "hash", "preimage", 123uL, null)

    @Before fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
        db = DatabaseService(context)
        db.saveChannel("channel", "7", 10.0, 11_000, null, 20_000, 100_000.0)
        restart()
    }

    @After fun tearDown() {
        db.close()
        context.deleteDatabase(File(Constants.userDataDir(context), "stablechannels.db").absolutePath)
    }

    @Test fun successWithClosedChannelAllowsTheFollowingCloseEventAndArchivesObligation() {
        pending()
        event(success)
        assertEquals("completed", payment().status)
        assertEquals(123L, payment().feeMsat)
        assertFalse(db.isLightningAccountingComplete("send")) // not falsely declared reconciled
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertFalse(db.hasPendingChannelSend())
        event(Event.ChannelClosed("channel", "7", null, null))
        assertEquals("Channel closed", state.statusMessage.value)
        assertArchive(10.0, 11_000L)
        event(success)
        assertEquals(1, db.getRecentPayments().count { it.paymentId == "send" })
        assertArchive(10.0, 11_000L)
    }

    @Test fun nativeOnlySuccessWithNoRemainingChannelDoesNotThrow() {
        db.saveChannel("channel", "7", 0.0, 0, null, 20_000, 0.0)
        setBooks(0.0, 0)
        pending()
        event(success)
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertArchive(0.0, 0L)
    }

    @Test fun restartRecoversCompletedSendWithoutChannelOrTrustedPrice() {
        pending()
        node.payments = listOf(terminal())
        call("reconcilePendingLightningPayments") // same recovery called by start()
        assertTrue(db.hasArchivedLightningAccounting("send"))
        db.close()
        db = DatabaseService(context)
        restart()
        call("reconcilePendingLightningPayments")
        call("runStabilityCheck")
        assertArchive(10.0, 11_000L)
        assertFalse(db.hasPendingChannelSend())
    }

    @Test fun archivedSuccessCannotReconcileAgainstNewChannel() {
        pending()
        event(success)
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 1_000))
        event(success)
        call("reconcilePendingLightningPayments")
        assertEquals(5.0, db.loadChannel("8")!!.expectedUSD, 0.0)
        assertEquals(5_000L, db.loadChannel("8")!!.backingSats)
        assertArchive(10.0, 11_000L)
    }

    @Test fun unreadyChannelIsDeferredAndNotArchivedThenRecoversAtReadyBalance() {
        pending()
        node.channels = listOf(channel(ready = false))
        event(success)
        assertTrue(db.hasPendingChannelSend())
        assertFalse(db.hasArchivedLightningAccounting("send"))
        assertNotNull(db.loadChannel("7"))
        node.channels = listOf(channel(receiver = 10_000))
        price(100_000.0)
        // The successful event is saved: no LDK payment record is required on retry.
        call("reconcilePendingLightningPayments")
        assertTrue(db.isLightningAccountingComplete("send"))
        assertEquals(9.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        call("reconcilePendingLightningPayments")
        assertEquals(9.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test fun pendingHtlcDoesNotBlockEventQueueOrGetMistakenForSettledSpend() {
        pending()
        node.channels = listOf(channel(receiver = 5_000))
        node.payments = listOf(terminal().copy(id = "other", status = PaymentStatus.PENDING))
        price(100_000.0)
        event(success)
        assertEquals("completed", payment().status)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertTrue(db.hasPendingChannelSend())
        node.payments = emptyList()
        node.channels = listOf(channel(receiver = 10_000))
        call("reconcilePendingLightningPayments")
        assertEquals(9.0, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test fun accountingFailureIsDeferredAndRetriedWithoutLosingAcknowledgementOrDoubleDebit() {
        pending()
        node.channels = listOf(channel(receiver = 10_000))
        price(100_000.0)
        db.writableDatabase.execSQL("CREATE TRIGGER reject_books BEFORE UPDATE ON channels BEGIN SELECT RAISE(ABORT, 'test'); END")
        event(success)
        assertEquals("completed", payment().status)
        assertTrue(db.hasPendingChannelSend())
        call("reconcilePendingLightningPayments")
        call("runStabilityCheck")
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        db.writableDatabase.execSQL("DROP TRIGGER reject_books")
        call("reconcilePendingLightningPayments")
        event(success)
        assertEquals(9.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(123L, payment().feeMsat)
    }

    @Test fun failureBeforeDurableHandoffMustStillLeaveEventUnacknowledged() {
        pending()
        db.writableDatabase.execSQL("CREATE TRIGGER reject_status BEFORE UPDATE OF status ON payments BEGIN SELECT RAISE(ABORT, 'test'); END")
        assertThrows(Exception::class.java) { event(success) }
        assertEquals("pending", payment().status)
        node.payments = listOf(terminal())
        // Startup recovery and the tick must isolate this same database failure.
        call("reconcilePendingLightningPayments")
        call("runStabilityCheck")
        assertEquals("pending", payment().status)
        db.writableDatabase.execSQL("DROP TRIGGER reject_status")
        event(success)
        assertTrue(db.hasArchivedLightningAccounting("send"))
    }

    @Test fun failedArchiveKeepsActiveBooksThenRetries() {
        pending()
        db.writableDatabase.execSQL("CREATE TRIGGER reject_archive BEFORE INSERT ON closed_channel_books BEGIN SELECT RAISE(ABORT, 'test'); END")
        event(success)
        assertNotNull(db.loadChannel("7"))
        assertTrue(db.hasPendingChannelSend())
        db.writableDatabase.execSQL("DROP TRIGGER reject_archive")
        call("reconcilePendingLightningPayments")
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertArchive(10.0, 11_000L)
    }

    @Test fun nativeSendWithBoundedFeesWorksWithoutPriceAndAccountsWithoutPrice() {
        node.channels = listOf(channel(receiver = 20_000))
        // 8,000 sats + 130 sats maximum routing fee fits the 9,000 native sats.
        val id = state.nodeService.sendTrackedLightningPayment("lightning", 8_000_000, null) { "send" }
        assertEquals("send", id)
        assertEquals(8_130L, state.nodeService.maximumLightningDebitSats(8_000_000))
        assertEquals(130_000uL, state.nodeService.routingParameters(8_000_000).maxTotalRoutingFeeMsat)
        node.channels = listOf(channel(receiver = 11_870))
        event(success)
        assertTrue(db.isLightningAccountingComplete("send"))
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(11_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test fun feeReachingSurplusIsBlockedBeforeLdkSubmission() {
        node.channels = listOf(channel(receiver = 20_000))
        price(100_000.0)
        var sent = false
        assertThrows(IllegalStateException::class.java) {
            state.nodeService.sendTrackedLightningPayment("bolt12", 9_000_000, null) {
                sent = true
                "send"
            }
        }
        assertFalse(sent)
        assertTrue(db.getRecentPayments().isEmpty())
    }

    @Test fun lspOwingTheShortfallDoesNotPreventSpendingStableBacking() {
        node.channels = listOf(channel(receiver = 20_000))
        price(50_000.0) // backing is worth $5.50, target $10: the LSP owes the client.
        assertEquals("send", state.nodeService.sendTrackedLightningPayment("bolt12", 15_000_000, null) { "send" })
    }

    @Test fun paymentBoundRoundsUpMsatsAndRejectsOverflow() {
        assertEquals(52L, state.nodeService.maximumLightningDebitSats(1_001L))
        assertNull(state.nodeService.maximumLightningDebitSats(0L))
        assertThrows(ArithmeticException::class.java) { state.nodeService.maximumLightningDebitSats(Long.MAX_VALUE) }
    }

    @Test fun migrationPreservesLegacyMarkerAndArchivesItWithTheClosingChannel() {
        db.recordPayment("send", "lightning", "sent", 9_000_000, status = "pending")
        db.writableDatabase.execSQL("DROP TABLE outgoing_lightning_accounting")
        db.writableDatabase.execSQL("CREATE TABLE outgoing_lightning_accounting (payment_id TEXT PRIMARY KEY, completed INTEGER NOT NULL DEFAULT 0)")
        db.writableDatabase.execSQL("INSERT INTO outgoing_lightning_accounting VALUES ('send', 0)")
        db.close()
        db = DatabaseService(context)
        restart()
        // ChannelClosed can arrive before the delayed success.
        event(Event.ChannelClosed("channel", "7", null, null))
        event(success)
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertArchive(10.0, 11_000L)
        assertEquals("completed", payment().status)
        assertFalse(db.hasPendingChannelSend())
    }

    @Test fun alreadyMissingChannelRowRetainsUnknownBooksAndOriginalPaymentIdentity() {
        pending()
        db.writableDatabase.execSQL("DELETE FROM channels")
        event(success)
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertFalse(db.hasPendingChannelSend())
        db.readableDatabase.rawQuery("SELECT expected_usd, stable_sats FROM closed_channel_books WHERE user_channel_id = '7'", null).use {
            assertTrue(it.moveToFirst())
            assertTrue(it.isNull(0)) // unknown obligation, never fabricated zero debt
            assertTrue(it.isNull(1))
        }
    }

    @Test fun stoppedNodeCannotArchiveAChannelFromAnEmptyList() {
        pending()
        @Suppress("UNCHECKED_CAST")
        val running = field(state.nodeService, "_isRunning").get(state.nodeService) as MutableStateFlow<Boolean>
        running.value = false
        event(success)
        assertTrue(db.hasPendingChannelSend())
        assertNotNull(db.loadChannel("7"))
        assertFalse(db.hasArchivedLightningAccounting("send"))
    }

    @Test fun closureDuringBalanceRefreshCannotCompleteAccountingFromOldDisplayedBooks() {
        pending()
        price(100_000.0)
        node.channelSnapshots = mutableListOf(listOf(channel(receiver = 10_000)), emptyList())
        event(success)
        assertFalse(db.isLightningAccountingComplete("send"))
        assertTrue(db.hasPendingChannelSend())
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        call("reconcilePendingLightningPayments")
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertArchive(10.0, 11_000L)
    }

    @Test fun ambiguousChannelSnapshotIsNotEvidenceOfClosure() {
        pending()
        node.channels = listOf(channel(), channel())
        event(success)
        assertFalse(db.hasArchivedLightningAccounting("send"))
        assertFalse(db.isLightningAccountingComplete("send"))
        assertTrue(db.hasPendingChannelSend())
        assertNotNull(db.loadChannel("7"))
    }

    @Test fun stabilitySuccessMustDebitOriginalArchiveAndNotReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        event(success)
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
        event(success)
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
    }

    @Test fun stabilityTickMustDebitOriginalArchiveAndNotReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        call("runStabilityCheck")
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
        call("runStabilityCheck")
        assertArchive(10.0, 10_000L)
    }

    @Test fun stabilityBackgroundRecoveryMustDebitOriginalArchiveAndNotReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        backgroundRecovery()
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
        backgroundRecovery()
        assertArchive(10.0, 10_000L)
    }

    @Test fun stabilityOriginSurvivesDatabaseReopenBeforeTerminalRecovery() {
        closeWithPendingStabilityAndReplace()
        db.close()
        db = DatabaseService(context)
        restart()
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        node.payments = listOf(terminal().copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL))
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        event(success)
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
    }

    @Test fun stabilityPendingOrFailedOutcomeNeverDebitsEitherChannelsBooks() {
        closeWithPendingStabilityAndReplace()
        node.payments = node.payments.map { it.copy(status = PaymentStatus.PENDING) }
        assertFalse(backgroundRecovery())
        assertNotNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
        node.payments = node.payments.map { it.copy(status = PaymentStatus.FAILED) }
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    @Test fun stabilityFailedEventClearsOnlyItsOriginalClaimWithoutADebit() {
        closeWithPendingStabilityAndReplace()
        event(Event.PaymentFailed("send", null, null))
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    @Test fun stabilityArchiveDebitHistoryAndMarkerRemovalRollBackTogether() {
        closeWithPendingStabilityAndReplace()
        db.writableDatabase.execSQL("CREATE TRIGGER reject_stability_clear BEFORE DELETE ON pending_stability_send BEGIN SELECT RAISE(ABORT, 'test'); END")
        event(success)
        assertNotNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L)
        assertFalse(db.isOutgoingStabilityPayment("send"))
        assertReplacementUnchanged()
        db.writableDatabase.execSQL("DROP TRIGGER reject_stability_clear")
        assertTrue(backgroundRecovery())
        event(success)
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertEquals(1, db.getRecentPayments().count { it.paymentId == "send" })
        db.readableDatabase.rawQuery("SELECT user_channel_id FROM outgoing_stability_accounting WHERE payment_id = 'send'", null).use {
            assertTrue(it.moveToFirst())
            assertEquals("7", it.getString(0))
        }
    }

    @Test fun stabilityLostPaymentIdRecoveryKeepsTheChannelSavedAtClaim() {
        closeWithPendingStabilityAndReplace()
        db.setPendingSendPaymentId("")
        node.payments = node.payments.map { it.copy(latestUpdateTimestamp = (System.currentTimeMillis() / 1000).toULong()) }
        event(success)
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
    }

    @Test fun stabilityLegacyMarkerWithoutOriginNeverAdoptsTheReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        db.writableDatabase.execSQL("DROP TABLE pending_stability_send")
        db.writableDatabase.execSQL("CREATE TABLE pending_stability_send (id INTEGER PRIMARY KEY, payment_id TEXT NOT NULL, amount_msat INTEGER NOT NULL, price REAL NOT NULL, created_at INTEGER NOT NULL)")
        db.writableDatabase.execSQL("INSERT INTO pending_stability_send VALUES (1, 'send', 1000000, 100000.0, strftime('%s','now'))")
        db.close()
        db = DatabaseService(context)
        field(state, "databaseService").set(state, db)
        assertNull(db.loadPendingSend()!!.userChannelId)
        event(success)
        assertFalse(backgroundRecovery())
        assertNotNull(db.loadPendingSend())
        assertReplacementUnchanged()
        assertArchive(10.0, 11_000L)
    }

    @Test fun stabilityBackgroundClaimPersistsItsExplicitOriginBeforePaymentId() {
        // The database has a newer channel too; claim the requested channel, never the newest row.
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        val controller = Robolectric.buildService(StabilityProcessingService::class.java).create()
        try {
            val claimed = StabilityProcessingService::class.java.getDeclaredMethod("claimPendingSendInDB",
                Long::class.javaPrimitiveType, Double::class.javaPrimitiveType, String::class.java)
                .apply { isAccessible = true }.invoke(controller.get(), 1_000_000L, 100_000.0, "7") as Boolean
            assertTrue(claimed)
            assertEquals("7", db.loadPendingSend()!!.userChannelId)
            assertEquals("", db.loadPendingSend()!!.paymentId)
        } finally { controller.destroy() }
    }

    @Test fun stabilityLiveChannelRecoveryDebitsExactlyOnce() {
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("send")
        node.channels = listOf(channel(receiver = 19_000))
        node.payments = listOf(terminal().copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL))
        event(success)
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(10_000L, state.stableChannel.value.backingSats)
        assertEquals(9_000L, state.stableChannel.value.nativeChannelBTC.sats)
        event(success)
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test fun stabilityOldReplayCannotRemoveOrCompleteANewerClaim() {
        closeWithPendingStabilityAndReplace()
        val oldClaim = db.loadPendingSend()!!
        event(success)
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "8"))
        db.setPendingSendPaymentId("new-send")
        node.payments = node.payments + terminal().copy(id = "new-send", kind = PaymentKind.Spontaneous("new-hash", null),
            amountMsat = 1_000_000uL, status = PaymentStatus.PENDING)
        event(success)
        assertFalse(db.clearPendingSend(oldClaim))
        assertFalse(db.completePendingStabilitySend(oldClaim, channelClosed = true))
        assertEquals("new-send", db.loadPendingSend()!!.paymentId)
        assertEquals("8", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
    }

    @Test fun stabilityUnknownArchivedBooksRecordThePaymentWithoutInventingBacking() {
        closeWithPendingStabilityAndReplace()
        db.writableDatabase.execSQL("UPDATE closed_channel_books SET stable_sats = NULL, expected_usd = NULL WHERE user_channel_id = '7'")
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertTrue(db.isOutgoingStabilityPayment("send"))
        assertReplacementUnchanged()
        db.readableDatabase.rawQuery("SELECT stable_sats FROM closed_channel_books WHERE user_channel_id = '7'", null).use {
            assertTrue(it.moveToFirst())
            assertTrue(it.isNull(0))
        }
    }

    @Test fun stabilityRecordedBeforeMarkerClearIsNotDebitedAgainAfterClosure() {
        closeWithPendingStabilityAndReplace()
        // Emulate the old atomic history/debit writer committing before the marker was cleared.
        db.writableDatabase.execSQL("UPDATE closed_channel_books SET stable_sats = 10000 WHERE user_channel_id = '7'")
        db.recordPayment("send", "stability", "sent", 1_000_000, status = "completed")
        assertTrue(backgroundRecovery())
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertNull(db.loadPendingSend())
    }

    @Test fun stabilityBackgroundArchivesOriginWhenTheClosureEventWasConsumedElsewhere() {
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("send")
        // LDK has only the replacement, while the unprocessed close left the old DB row behind.
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        node.payments = listOf(terminal().copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL))
        assertTrue(backgroundRecovery())
        assertNull(db.loadChannel("7"))
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertNull(db.loadPendingSend())
    }

    @Test fun stabilityMissingLdkRecordKeepsItsOriginalUnresolvedClaim() {
        closeWithPendingStabilityAndReplace()
        node.payments = emptyList()
        assertFalse(backgroundRecovery())
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    @Test fun stabilityAmbiguousLostIdDoesNotChooseOneOfSeveralKeysends() {
        closeWithPendingStabilityAndReplace()
        db.setPendingSendPaymentId("")
        val candidate = node.payments.single().copy(latestUpdateTimestamp = (System.currentTimeMillis() / 1000).toULong())
        node.payments = listOf(candidate, candidate.copy(id = "another-send"))
        assertFalse(backgroundRecovery())
        assertEquals("", db.loadPendingSend()!!.paymentId)
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    private fun closeWithPendingStabilityAndReplace() {
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("send")
        node.payments = listOf(terminal().copy(kind = PaymentKind.Spontaneous("hash", null),
            amountMsat = 1_000_000uL, status = PaymentStatus.PENDING))
        event(Event.ChannelClosed("channel", "7", null, null))
        assertNotNull(db.loadPendingSend())
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        node.payments = node.payments.map { it.copy(status = PaymentStatus.SUCCEEDED) }
        price(100_000.0)
    }

    private fun assertReplacementUnchanged() {
        assertEquals(5_000L, db.loadChannel("8")!!.backingSats)
        assertEquals(5.0, db.loadChannel("8")!!.expectedUSD, 0.0)
        assertEquals(5_000L, state.stableChannel.value.backingSats)
    }

    private fun backgroundRecovery(): Boolean {
        val controller = Robolectric.buildService(StabilityProcessingService::class.java).create()
        try {
            return StabilityProcessingService::class.java.getDeclaredMethod("reconcilePendingOutgoingPayment",
                Node::class.java, String::class.java).apply { isAccessible = true }
                .invoke(controller.get(), node, db.writableDatabase.path) as Boolean
        } catch (e: InvocationTargetException) { throw e.targetException }
        finally { controller.destroy() }
    }

    private fun pending() = db.recordPendingLightningPayment("send", "lightning", 9_000_000, 100_000.0, "7")
    private fun payment() = db.getRecentPayments().single { it.paymentId == "send" }
    private fun terminal() = PaymentDetails("send", PaymentKind.Bolt11("hash", null, null, null),
        9_000_000uL, 123uL, PaymentDirection.OUTBOUND, PaymentStatus.SUCCEEDED, 0uL)

    @Suppress("UNCHECKED_CAST")
    private fun restart() {
        state = AppState(context)
        field(state, "databaseService").set(state, db)
        node = TestNode()
        field(state.nodeService, "node").set(state.nodeService, node)
        (field(state.nodeService, "_isRunning").get(state.nodeService) as MutableStateFlow<Boolean>).value = true
        setBooks(10.0, 11_000)
    }

    @Suppress("UNCHECKED_CAST")
    private fun setBooks(expected: Double, backing: Long, uid: String = "7", cid: String = "channel") {
        (field(state, "_stableChannel").get(state) as MutableStateFlow<StableChannel>).value = StableChannel(
            userChannelId = uid, channelId = cid, expectedUSD = USD(expected), backingSats = backing,
            stableReceiverBTC = Bitcoin(20_000), latestPrice = 100_000.0)
    }

    @Suppress("UNCHECKED_CAST")
    private fun price(value: Double) {
        state.priceService.seedPrice(value)
        (field(state.priceService, "_lastUpdate").get(state.priceService) as MutableStateFlow<Date>).value = Date()
    }

    private fun field(target: Any, name: String) = target.javaClass.getDeclaredField(name).apply { isAccessible = true }
    private fun call(name: String) = invoke(name, emptyArray(), emptyArray())
    private fun event(event: Event) = invoke("handleEvent", arrayOf(Event::class.java), arrayOf(event))
    private fun invoke(name: String, types: Array<Class<*>>, args: Array<Any?>) {
        try { AppState::class.java.getDeclaredMethod(name, *types).apply { isAccessible = true }.invoke(state, *args) }
        catch (e: InvocationTargetException) { throw e.targetException }
    }

    private fun assertArchive(expected: Double, backing: Long) {
        db.readableDatabase.rawQuery("SELECT expected_usd, stable_sats FROM closed_channel_books WHERE user_channel_id = '7'", null).use {
            assertTrue(it.moveToFirst())
            assertEquals(expected, it.getDouble(0), 0.0)
            assertEquals(backing, it.getLong(1))
        }
    }

    // UniFFI uses Android's cleaner even for a NoPointer transport. Supply a JVM cleaner
    // so these tests need neither a native node nor JDK module-export flags.
    @Implements(className = "android.system.SystemCleaner", isInAndroidSdk = false)
    class SystemCleanerShadow {
        companion object {
            @JvmStatic @Implementation
            fun cleaner(): java.lang.ref.Cleaner = java.lang.ref.Cleaner.create()
        }
    }

    private class TestNode : Node(NoPointer) {
        var channels = emptyList<ChannelDetails>()
        var payments = emptyList<PaymentDetails>()
        var channelSnapshots = mutableListOf<List<ChannelDetails>>()
        override fun listChannels(): List<ChannelDetails> {
            if (channelSnapshots.isNotEmpty()) return channelSnapshots.removeAt(0)
            return channels
        }
        override fun listPayments() = payments
        override fun payment(paymentId: String) = payments.singleOrNull { it.id == paymentId }
        override fun listBalances() = BalanceDetails(0uL, 0uL, 0uL,
            channels.sumOf { it.outboundCapacityMsat / 1000uL }, emptyList(), emptyList())
    }

    private fun channel(uid: String = "7", cid: String = "channel", receiver: Long = 20_000, ready: Boolean = true) = ChannelDetails(
        channelId = cid, counterpartyNodeId = "peer", fundingTxo = null, fundingRedeemScript = null,
        shortChannelId = null, outboundScidAlias = null, inboundScidAlias = null,
        channelValueSats = 50_000uL, unspendablePunishmentReserve = 0uL, userChannelId = uid,
        feerateSatPer1000Weight = 0u, outboundCapacityMsat = receiver.toULong() * 1000uL,
        inboundCapacityMsat = 0uL, confirmationsRequired = null, confirmations = null,
        isOutbound = true, isChannelReady = ready, isUsable = ready, isAnnounced = false,
        cltvExpiryDelta = null, counterpartyUnspendablePunishmentReserve = 0uL,
        counterpartyOutboundHtlcMinimumMsat = null, counterpartyOutboundHtlcMaximumMsat = null,
        counterpartyForwardingInfoFeeBaseMsat = null, counterpartyForwardingInfoFeeProportionalMillionths = null,
        counterpartyForwardingInfoCltvExpiryDelta = null, nextOutboundHtlcLimitMsat = 0uL,
        nextOutboundHtlcMinimumMsat = 0uL, forceCloseSpendDelay = null, inboundHtlcMinimumMsat = 0uL,
        inboundHtlcMaximumMsat = null, config = ChannelConfig(0u, 0u, 0u, MaxDustHtlcExposure.FixedLimit(0uL), 0uL, false), channelShutdownState = null
    )
}
