package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.LightningPaymentRecovery
import com.stablechannels.app.util.Constants
import java.io.File
import java.lang.reflect.InvocationTargetException
import java.util.Date
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

/** Real AppState event/recovery entry points and SQLite; only the LDK transport is substituted. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], shadows = [SystemCleanerShadow::class])
class OutgoingPaymentLifecycleTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private lateinit var state: AppState
    private lateinit var node: TestNode
    private val success = Event.PaymentSuccessful("send", "hash", "preimage", 123uL, null)

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        auditFile.delete()
        AuditService.setLogPath(auditFile.path)
        db = DatabaseService(context)
        db.saveChannel("channel", "7", 10.0, 11_000, null, 20_000, 100_000.0)
        restart()
    }

    @After
    fun tearDown() {
        db.close()
        auditFile.delete()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    private val auditFile
        get() = File(context.cacheDir, "lifecycle-audit.log")

    private fun auditLog() = auditFile.takeIf { it.exists() }?.readText() ?: ""

    @Test
    fun successWithClosedChannelAllowsTheFollowingCloseEventAndArchivesObligation() {
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

    @Test
    fun nativeOnlySuccessWithNoRemainingChannelDoesNotThrow() {
        db.saveChannel("channel", "7", 0.0, 0, null, 20_000, 0.0)
        setBooks(0.0, 0)
        pending()
        event(success)
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertArchive(0.0, 0L)
    }

    @Test
    fun restartRecoversCompletedSendWithoutChannelOrTrustedPrice() {
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

    @Test
    fun pendingSendUnknownToLdkStopsBlockingSpendsAfterTheGracePeriod() {
        pending()
        node.channels = listOf(channel())
        price(100_000.0)
        call("reconcilePendingLightningPayments")
        assertTrue(db.hasPendingChannelSend())
        db.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - 601 WHERE payment_id = 'send'"
        )
        call("reconcilePendingLightningPayments")
        assertFalse(db.hasPendingChannelSend())
        assertEquals("failed", payment().status)
        assertEquals(
            "after-release",
            state.nodeService.sendTrackedLightningPayment("lightning", 1_000_000, null) {
                "after-release"
            },
        )
    }

    @Test
    fun archivedSuccessCannotReconcileAgainstNewChannel() {
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

    @Test
    fun unreadyChannelIsDeferredAndNotArchivedThenRecoversAtReadyBalance() {
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

    @Test
    fun pendingHtlcDoesNotBlockEventQueueOrGetMistakenForSettledSpend() {
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

    @Test
    fun accountingFailureIsDeferredAndRetriedWithoutLosingAcknowledgementOrDoubleDebit() {
        pending()
        node.channels = listOf(channel(receiver = 10_000))
        price(100_000.0)
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_books BEFORE UPDATE ON channels BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
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

    @Test
    fun failureBeforeDurableHandoffMustStillLeaveEventUnacknowledged() {
        pending()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_status BEFORE UPDATE OF status ON payments BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
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

    @Test
    fun failedArchiveKeepsActiveBooksThenRetries() {
        pending()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_archive BEFORE INSERT ON closed_channel_books BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        event(success)
        assertNotNull(db.loadChannel("7"))
        assertTrue(db.hasPendingChannelSend())
        db.writableDatabase.execSQL("DROP TRIGGER reject_archive")
        call("reconcilePendingLightningPayments")
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertArchive(10.0, 11_000L)
    }

    @Test
    fun nativeSendWithBoundedFeesWorksWithoutPriceAndAccountsWithoutPrice() {
        node.channels = listOf(channel(receiver = 20_000))
        // 8,000 sats + 130 sats maximum routing fee fits the 9,000 native sats.
        val id =
            state.nodeService.sendTrackedLightningPayment("lightning", 8_000_000, null) { "send" }
        assertEquals("send", id)
        assertEquals(8_130L, state.nodeService.maximumLightningDebitSats(8_000_000))
        assertEquals(
            130_000uL,
            state.nodeService.routingParameters(8_000_000).maxTotalRoutingFeeMsat,
        )
        node.channels = listOf(channel(receiver = 11_870))
        event(success)
        assertTrue(db.isLightningAccountingComplete("send"))
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(11_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun feeReachingSurplusIsBlockedBeforeLdkSubmission() {
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

    @Test
    fun lspOwingTheShortfallDoesNotPreventSpendingStableBacking() {
        node.channels = listOf(channel(receiver = 20_000))
        price(50_000.0) // backing is worth $5.50, target $10: the LSP owes the client.
        assertEquals(
            "send",
            state.nodeService.sendTrackedLightningPayment("bolt12", 15_000_000, null) { "send" },
        )
    }

    @Test
    fun paymentBoundRoundsUpMsatsAndRejectsOverflow() {
        assertEquals(52L, state.nodeService.maximumLightningDebitSats(1_001L))
        assertNull(state.nodeService.maximumLightningDebitSats(0L))
        assertThrows(ArithmeticException::class.java) {
            state.nodeService.maximumLightningDebitSats(Long.MAX_VALUE)
        }
    }

    @Test
    fun migrationPreservesLegacyMarkerAndArchivesItWithTheClosingChannel() {
        db.recordPayment("send", "lightning", "sent", 9_000_000, status = "pending")
        db.writableDatabase.execSQL("DROP TABLE outgoing_lightning_accounting")
        db.writableDatabase.execSQL(
            "CREATE TABLE outgoing_lightning_accounting (payment_id TEXT PRIMARY KEY, completed INTEGER NOT NULL DEFAULT 0)"
        )
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

    @Test
    fun alreadyMissingChannelRowRetainsUnknownBooksAndOriginalPaymentIdentity() {
        pending()
        db.writableDatabase.execSQL("DELETE FROM channels")
        event(success)
        assertTrue(db.hasArchivedLightningAccounting("send"))
        assertFalse(db.hasPendingChannelSend())
        db.readableDatabase
            .rawQuery(
                "SELECT expected_usd, stable_sats FROM closed_channel_books WHERE user_channel_id = '7'",
                null,
            )
            .use {
                assertTrue(it.moveToFirst())
                assertTrue(it.isNull(0)) // unknown obligation, never fabricated zero debt
                assertTrue(it.isNull(1))
            }
    }

    @Test
    fun stoppedNodeCannotArchiveAChannelFromAnEmptyList() {
        pending()
        @Suppress("UNCHECKED_CAST")
        val running =
            field(state.nodeService, "_isRunning").get(state.nodeService)
                as MutableStateFlow<Boolean>
        running.value = false
        event(success)
        assertTrue(db.hasPendingChannelSend())
        assertNotNull(db.loadChannel("7"))
        assertFalse(db.hasArchivedLightningAccounting("send"))
    }

    @Test
    fun closureDuringBalanceRefreshCannotCompleteAccountingFromOldDisplayedBooks() {
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

    @Test
    fun ambiguousChannelSnapshotIsNotEvidenceOfClosure() {
        pending()
        node.channels = listOf(channel(), channel())
        event(success)
        assertFalse(db.hasArchivedLightningAccounting("send"))
        assertFalse(db.isLightningAccountingComplete("send"))
        assertTrue(db.hasPendingChannelSend())
        assertNotNull(db.loadChannel("7"))
    }

    @Test
    fun stabilitySuccessMustDebitOriginalArchiveAndNotReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        event(success)
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
        event(success)
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
    }

    @Test
    fun stabilityTickMustDebitOriginalArchiveAndNotReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        call("runStabilityCheck")
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
        call("runStabilityCheck")
        assertArchive(10.0, 10_000L)
    }

    @Test
    fun stabilityBackgroundRecoveryMustDebitOriginalArchiveAndNotReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        backgroundRecovery()
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
        backgroundRecovery()
        assertArchive(10.0, 10_000L)
    }

    @Test
    fun stabilityOriginSurvivesDatabaseReopenBeforeTerminalRecovery() {
        closeWithPendingStabilityAndReplace()
        db.close()
        db = DatabaseService(context)
        restart()
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        node.payments =
            listOf(
                terminal()
                    .copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL)
            )
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        event(success)
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
    }

    @Test
    fun stabilityPendingOrFailedOutcomeNeverDebitsEitherChannelsBooks() {
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

    @Test
    fun stabilityFailedEventClearsOnlyItsOriginalClaimWithoutADebit() {
        closeWithPendingStabilityAndReplace()
        event(Event.PaymentFailed("send", null, null))
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    @Test
    fun stabilityArchiveDebitHistoryAndMarkerRemovalRollBackTogether() {
        closeWithPendingStabilityAndReplace()
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_stability_clear BEFORE DELETE ON pending_stability_send BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
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
        db.readableDatabase
            .rawQuery(
                "SELECT user_channel_id FROM outgoing_stability_accounting WHERE payment_id = 'send'",
                null,
            )
            .use {
                assertTrue(it.moveToFirst())
                assertEquals("7", it.getString(0))
            }
    }

    @Test
    fun stabilityLostPaymentIdRecoveryKeepsTheChannelSavedAtClaim() {
        closeWithPendingStabilityAndReplace()
        db.setPendingSendPaymentId("")
        node.payments =
            node.payments.map {
                it.copy(latestUpdateTimestamp = (System.currentTimeMillis() / 1000).toULong())
            }
        event(success)
        assertReplacementUnchanged()
        assertArchive(10.0, 10_000L)
        assertNull(db.loadPendingSend())
    }

    @Test
    fun stabilityLegacyMarkerWithoutOriginReleasesWithoutAdoptingTheReplacementChannel() {
        closeWithPendingStabilityAndReplace()
        reopenLegacyStabilityMarker()
        assertNull(db.loadPendingSend()!!.userChannelId)
        event(success)
        assertNull(db.loadPendingSend())
        call("runStabilityCheck")
        assertTrue(backgroundRecovery())
        assertReplacementUnchanged()
        assertArchive(10.0, 11_000L)
        assertLegacyPaymentOnRecordWithoutOrigin()
    }

    @Test
    fun stabilityAccountedLegacyMarkerReleasesNativeSpendAfterSuccessEvent() {
        prepareAccountedLegacyStability()
        event(success)
        assertNull(db.loadPendingSend())
        event(success) // A duplicate success must not debit the already-accounted payment.
        assertLegacyAccountingUnchanged()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityAccountedLegacyMarkerReleasesNativeSpendAfterTick() {
        prepareAccountedLegacyStability()
        call("runStabilityCheck")
        assertNull(db.loadPendingSend())
        call("runStabilityCheck")
        assertLegacyAccountingUnchanged()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityAccountedLegacyMarkerReleasesNativeSpendAfterBackgroundRecovery() {
        prepareAccountedLegacyStability()
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertTrue(backgroundRecovery())
        assertLegacyAccountingUnchanged()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityAccountedLegacyMarkerReleasesSpliceStart() {
        prepareAccountedLegacyStability()
        val blocked =
            assertThrows(IllegalStateException::class.java) {
                state.beginSpliceOut(1_000, "test-address", 100_000.0)
            }
        assertTrue(blocked.message!!.contains("Waiting for the previous payment"))
        assertFalse(db.hasPendingSplice())
        assertTrue(backgroundRecovery())
        state.beginSpliceOut(1_000, "test-address", 100_000.0)
        assertTrue(db.hasPendingSplice())
        assertLegacyAccountingUnchanged()
    }

    @Test
    fun stabilityAccountedLegacyMarkerNeverChangesReplacementOrArchivedBooks() {
        prepareAccountedLegacyStability()
        node.channels = emptyList()
        event(Event.ChannelClosed("channel", "7", null, null))
        assertArchive(10.0, 10_000L)
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        assertTrue(backgroundRecovery())
        event(success)
        call("runStabilityCheck")
        assertNull(db.loadPendingSend())
        assertNull(db.loadChannel("7"))
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertNoStabilityOriginRecorded()
    }

    @Test
    fun stabilityLegacyCleanupRequiresMatchingCompletedOutgoingStabilityHistory() {
        prepareAccountedLegacyStability()
        val pending = db.loadPendingSend()!!
        val restore =
            "UPDATE payments SET payment_id = 'send', payment_type = 'stability', direction = 'sent', status = 'completed', amount_msat = 1000000"
        for (mismatch in
            listOf(
                "payment_id = 'another-send'",
                "payment_type = 'lightning'",
                "direction = 'received'",
                "status = 'pending'",
                "status = 'failed'",
                "amount_msat = 999000",
            )) {
            db.writableDatabase.execSQL(restore)
            db.writableDatabase.execSQL("UPDATE payments SET $mismatch")
            assertFalse(mismatch, db.clearAccountedLegacyStabilitySend(pending))
            assertEquals(pending, db.loadPendingSend())
            assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        }
        // Books already at par reproduce no claim, so an unmatched row is released as
        // unattributed: the payment stays on record and nothing is debited again.
        db.writableDatabase.execSQL(restore)
        db.writableDatabase.execSQL("UPDATE payments SET status = 'pending'")
        assertTrue(backgroundRecovery())
        assertLegacyAccountingUnchanged()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityLegacyCompletedHistoryStillRequiresMatchingTransportSuccess() {
        prepareAccountedLegacyStability()
        val pending = db.loadPendingSend()!!
        val succeeded = node.payments.single()
        val unresolved =
            listOf(
                null,
                succeeded.copy(status = PaymentStatus.PENDING),
                succeeded.copy(id = "another-send"),
                succeeded.copy(amountMsat = 999_000uL),
                succeeded.copy(direction = PaymentDirection.INBOUND),
                succeeded.copy(kind = PaymentKind.Bolt11("hash", null, null, null)),
            )
        for (payment in unresolved) {
            node.payments = listOfNotNull(payment)
            assertFalse(backgroundRecovery())
            assertEquals(pending, db.loadPendingSend())
            assertLegacyAccountingUnchanged()
        }
        node.payments = listOf(succeeded)
        assertTrue(backgroundRecovery())
        assertLegacyAccountingUnchanged()
    }

    @Test
    fun stabilityLegacyCleanupRetriesAfterDatabaseFailureWithoutAnotherDebit() {
        prepareAccountedLegacyStability()
        val pending = db.loadPendingSend()!!
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_legacy_clear BEFORE DELETE ON pending_stability_send BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        event(success)
        assertEquals(pending, db.loadPendingSend())
        assertLegacyAccountingUnchanged()
        db.writableDatabase.execSQL("DROP TRIGGER reject_legacy_clear")
        assertTrue(backgroundRecovery())
        event(success)
        assertNull(db.loadPendingSend())
        assertLegacyAccountingUnchanged()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerDebitsTheOnlyChannelSavedBeforeTheClaimAfterSuccessEvent() {
        prepareUnaccountedLegacyStability()
        event(success)
        assertLegacyDebitAppliedOnce()
        assertEquals(10_000L, state.stableChannel.value.backingSats)
        event(success)
        assertLegacyDebitAppliedOnce()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerDebitsTheOnlyChannelSavedBeforeTheClaimAfterTick() {
        prepareUnaccountedLegacyStability()
        call("runStabilityCheck")
        assertLegacyDebitAppliedOnce()
        assertEquals(10_000L, state.stableChannel.value.backingSats)
        call("runStabilityCheck")
        assertLegacyDebitAppliedOnce()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerDebitsTheOnlyChannelSavedBeforeTheClaimInTheBackground() {
        prepareUnaccountedLegacyStability()
        assertTrue(backgroundRecovery())
        assertLegacyDebitAppliedOnce()
        assertTrue(backgroundRecovery())
        assertLegacyDebitAppliedOnce()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerStillRequiresMatchingTransportSuccess() {
        prepareUnaccountedLegacyStability()
        val pending = db.loadPendingSend()!!
        val succeeded = node.payments.single()
        val unresolved =
            listOf(
                null,
                succeeded.copy(status = PaymentStatus.PENDING),
                succeeded.copy(id = "another-send"),
                succeeded.copy(amountMsat = 999_000uL),
                succeeded.copy(direction = PaymentDirection.INBOUND),
                succeeded.copy(kind = PaymentKind.Bolt11("hash", null, null, null)),
            )
        for (payment in unresolved) {
            node.payments = listOfNotNull(payment)
            assertFalse(backgroundRecovery())
            assertEquals(pending, db.loadPendingSend())
            assertLegacyBooksUntouched()
        }
        node.payments = listOf(succeeded)
        assertTrue(backgroundRecovery())
        assertLegacyDebitAppliedOnce()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerSurvivesADatabaseFailureAndDebitsOnce() {
        prepareUnaccountedLegacyStability()
        val pending = db.loadPendingSend()!!
        db.writableDatabase.execSQL(
            "CREATE TRIGGER reject_legacy_clear BEFORE DELETE ON pending_stability_send BEGIN SELECT RAISE(ABORT, 'test'); END"
        )
        event(success)
        // The proven origin is bound durably even though the debit and clear rolled back.
        assertEquals(pending.copy(userChannelId = "7"), db.loadPendingSend())
        assertLegacyBooksUntouched()
        db.writableDatabase.execSQL("DROP TRIGGER reject_legacy_clear")
        event(success)
        assertLegacyDebitAppliedOnce()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerReleasesWithoutChargingAReplacementOpenedAfterTheClaim() {
        // Before the archive existed, closing the origin deleted its row; a replacement is always
        // newer than the claim.
        db.writableDatabase.execSQL("DELETE FROM channels WHERE user_channel_id = '7'")
        reopenLegacyStabilityMarker(claimAgeSecs = 600)
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        restart()
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        node.payments =
            listOf(
                terminal()
                    .copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL)
            )
        price(100_000.0)
        event(success)
        assertNull(db.loadPendingSend())
        assertReplacementUnchanged()
        assertLegacyPaymentOnRecordWithoutOrigin()
        call("runStabilityCheck")
        assertTrue(backgroundRecovery())
        assertReplacementUnchanged()
        assertLegacyPaymentOnRecordWithoutOrigin()
        assertEquals(
            "after-release",
            state.nodeService.sendTrackedLightningPayment("lightning", 1_000_000, null) {
                "after-release"
            },
        )
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerWithSeveralOlderChannelsDebitsNeither() {
        db.saveChannel("other-channel", "9", 0.0, 0, null, 1_000, 0.0)
        prepareUnaccountedLegacyStability()
        node.channels = listOf(channel(receiver = 19_000), channel("9", "other-channel", 1_000))
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertEquals(11_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(0L, db.loadChannel("9")!!.backingSats)
        assertLegacyPaymentOnRecordWithoutOrigin()
        assertTrue(auditLog().contains("STABILITY_LEGACY_UNATTRIBUTED"))
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityUnaccountedLegacyMarkerDebitsNothingWhenTheBooksDoNotReproduceTheClaim() {
        // Books worth $11 against a $10 target claim exactly $1; a $2 marker did not come from
        // them.
        prepareUnaccountedLegacyStability(amountMsat = 2_000_000)
        event(success)
        assertNull(db.loadPendingSend())
        assertEquals(11_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(2_000_000L, payment().amountMsat)
        assertLegacyPaymentOnRecordWithoutOrigin()
        assertNativeSpendAfterLegacyRecovery()
    }

    @Test
    fun stabilityLegacyCleanupCannotClearANewerClaimWithTheSameAmount() {
        prepareAccountedLegacyStability()
        val oldClaim = db.loadPendingSend()!!
        assertTrue(backgroundRecovery())
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("new-send")
        val newClaim = db.loadPendingSend()!!
        node.payments =
            node.payments +
                node.payments.single().copy(id = "new-send", status = PaymentStatus.PENDING)
        assertFalse(db.clearAccountedLegacyStabilitySend(oldClaim))
        event(success)
        assertEquals(newClaim, db.loadPendingSend())
        assertLegacyAccountingUnchanged()
    }

    @Test
    fun stabilityBackgroundClaimPersistsItsExplicitOriginBeforePaymentId() {
        // The database has a newer channel too; claim the requested channel, never the newest row.
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        val controller = Robolectric.buildService(StabilityProcessingService::class.java).create()
        try {
            val claimed =
                StabilityProcessingService::class
                    .java
                    .getDeclaredMethod(
                        "claimPendingSendInDB",
                        Long::class.javaPrimitiveType,
                        Double::class.javaPrimitiveType,
                        String::class.java,
                    )
                    .apply { isAccessible = true }
                    .invoke(controller.get(), 1_000_000L, 100_000.0, "7") as Boolean
            assertTrue(claimed)
            assertEquals("7", db.loadPendingSend()!!.userChannelId)
            assertEquals("", db.loadPendingSend()!!.paymentId)
        } finally {
            controller.destroy()
        }
    }

    @Test
    fun stabilityLiveChannelRecoveryDebitsExactlyOnce() {
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("send")
        node.channels = listOf(channel(receiver = 19_000))
        node.payments =
            listOf(
                terminal()
                    .copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL)
            )
        event(success)
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(10_000L, state.stableChannel.value.backingSats)
        assertEquals(9_000L, state.stableChannel.value.nativeChannelBTC.sats)
        event(success)
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun stabilityOldReplayCannotRemoveOrCompleteANewerClaim() {
        closeWithPendingStabilityAndReplace()
        val oldClaim = db.loadPendingSend()!!
        event(success)
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "8"))
        db.setPendingSendPaymentId("new-send")
        node.payments =
            node.payments +
                terminal()
                    .copy(
                        id = "new-send",
                        kind = PaymentKind.Spontaneous("new-hash", null),
                        amountMsat = 1_000_000uL,
                        status = PaymentStatus.PENDING,
                    )
        event(success)
        assertFalse(db.clearPendingSend(oldClaim))
        assertFalse(db.completePendingStabilitySend(oldClaim, channelClosed = true))
        assertEquals("new-send", db.loadPendingSend()!!.paymentId)
        assertEquals("8", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
    }

    @Test
    fun stabilityUnknownArchivedBooksRecordThePaymentWithoutInventingBacking() {
        closeWithPendingStabilityAndReplace()
        db.writableDatabase.execSQL(
            "UPDATE closed_channel_books SET stable_sats = NULL, expected_usd = NULL WHERE user_channel_id = '7'"
        )
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertTrue(db.isOutgoingStabilityPayment("send"))
        assertReplacementUnchanged()
        db.readableDatabase
            .rawQuery(
                "SELECT stable_sats FROM closed_channel_books WHERE user_channel_id = '7'",
                null,
            )
            .use {
                assertTrue(it.moveToFirst())
                assertTrue(it.isNull(0))
            }
    }

    @Test
    fun stabilityRecordedBeforeMarkerClearIsNotDebitedAgainAfterClosure() {
        closeWithPendingStabilityAndReplace()
        // Emulate the old atomic history/debit writer committing before the marker was cleared.
        db.writableDatabase.execSQL(
            "UPDATE closed_channel_books SET stable_sats = 10000 WHERE user_channel_id = '7'"
        )
        db.recordPayment("send", "stability", "sent", 1_000_000, status = "completed")
        assertTrue(backgroundRecovery())
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertNull(db.loadPendingSend())
    }

    @Test
    fun stabilityBackgroundArchivesOriginWhenTheClosureEventWasConsumedElsewhere() {
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("send")
        // LDK has only the replacement, while the unprocessed close left the old DB row behind.
        db.saveChannel("new-channel", "8", 5.0, 5_000, null, 5_000, 100_000.0)
        setBooks(5.0, 5_000, "8", "new-channel")
        node.channels = listOf(channel("8", "new-channel", 5_000))
        node.payments =
            listOf(
                terminal()
                    .copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL)
            )
        assertTrue(backgroundRecovery())
        assertNull(db.loadChannel("7"))
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertNull(db.loadPendingSend())
    }

    @Test
    fun stabilityMissingLdkRecordKeepsItsClaimWhileTheGracePeriodRuns() {
        closeWithPendingStabilityAndReplace()
        node.payments = emptyList()
        assertFalse(backgroundRecovery())
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    @Test
    fun stabilityMissingLdkRecordReleasesTheBarrierAfterTheGracePeriod() {
        closeWithPendingStabilityAndReplace()
        node.payments = emptyList()
        ageStabilityMarker()
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L) // nothing proves the payment left, so nothing is debited
        assertReplacementUnchanged()
        assertNoStabilityOriginRecorded()
        assertTrue(auditLog().contains("STABILITY_MARKER_RELEASED_NO_LDK_RECORD"))
        assertEquals(
            "after-release",
            state.nodeService.sendTrackedLightningPayment("lightning", 1_000_000, null) {
                "after-release"
            },
        )
    }

    @Test
    fun stabilityLateSuccessAfterALostRecordReleaseIsNeverAnOrdinarySend() {
        closeWithPendingStabilityAndReplace()
        val succeeded = node.payments.single()
        node.payments = emptyList()
        ageStabilityMarker()
        assertTrue(backgroundRecovery())
        // The released id stays on record as a stability payment whose outcome is unknown.
        assertTrue(db.isOutgoingStabilityPayment("send"))
        assertEquals("failed", payment().status)
        assertEquals(1_000_000L, payment().amountMsat)
        // A balance below backing would make an ordinary-send reconcile cut the USD target.
        node.channels = listOf(channel("8", "new-channel", 4_000))
        node.payments = listOf(succeeded)
        event(success)
        assertEquals("completed", payment().status)
        assertEquals(1, db.getRecentPayments().count { it.paymentId == "send" })
        assertEquals(5.0, db.loadChannel("8")!!.expectedUSD, 0.0)
        assertEquals(5_000L, db.loadChannel("8")!!.backingSats)
        assertArchive(10.0, 11_000L)
    }

    @Suppress("UNCHECKED_CAST")
    @Test
    fun stabilityMissingLdkRecordIsNeverReleasedByANodeThatIsNotRunning() {
        closeWithPendingStabilityAndReplace()
        node.payments = emptyList()
        ageStabilityMarker()
        (field(state.nodeService, "_isRunning").get(state.nodeService) as MutableStateFlow<Boolean>)
            .value = false
        call("runStabilityCheck")
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 11_000L)
    }

    private fun ageStabilityMarker() =
        db.writableDatabase.execSQL(
            "UPDATE pending_stability_send SET created_at = created_at - " +
                "${LightningPaymentRecovery.LOST_LDK_RECORD_TIMEOUT_SECS + 1}"
        )

    @Test
    fun stabilityAmbiguousLostIdAdoptsWhenEveryCandidateAgreesOnTheOutcome() {
        val candidate = ambiguousLostIdCandidate()
        node.payments =
            listOf(
                candidate.copy(
                    id = "later-send",
                    latestUpdateTimestamp = candidate.latestUpdateTimestamp + 5u,
                ),
                candidate,
            )
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 10_000L) // the debit is the same whichever keysend was ours
        assertReplacementUnchanged()
        assertEquals("7", db.outgoingStabilityOrigin("send")) // the earliest candidate is adopted
    }

    @Test
    fun stabilityAmbiguousLostIdWaitsWhileAnyCandidateIsStillInFlight() {
        val candidate = ambiguousLostIdCandidate()
        node.payments =
            listOf(candidate, candidate.copy(id = "another-send", status = PaymentStatus.PENDING))
        assertFalse(backgroundRecovery())
        assertEquals("", db.loadPendingSend()!!.paymentId)
        assertEquals("7", db.loadPendingSend()!!.userChannelId)
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    @Test
    fun stabilityAmbiguousLostIdWithConflictingOutcomesReleasesWithoutADebit() {
        val candidate = ambiguousLostIdCandidate()
        node.payments =
            listOf(candidate, candidate.copy(id = "another-send", status = PaymentStatus.FAILED))
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
        assertNoStabilityOriginRecorded()
        assertTrue(auditLog().contains("STABILITY_MARKER_RELEASED_AMBIGUOUS"))
    }

    @Test
    fun stabilityAmbiguousLostIdNeverAdoptsATradeFeeKeysend() {
        val candidate = ambiguousLostIdCandidate()
        recordTradeFeePayment("fee")
        node.payments =
            listOf(
                candidate.copy(
                    id = "fee",
                    latestUpdateTimestamp = candidate.latestUpdateTimestamp - 3u,
                ),
                candidate,
            )
        assertTrue(backgroundRecovery())
        assertNull(db.loadPendingSend())
        assertArchive(10.0, 10_000L)
        assertReplacementUnchanged()
        assertEquals("7", db.outgoingStabilityOrigin("send"))
        assertNull(db.outgoingStabilityOrigin("fee"))
    }

    @Test
    fun stabilityLostIdWithOnlyATradeFeeKeysendKeepsWaiting() {
        val candidate = ambiguousLostIdCandidate()
        recordTradeFeePayment("fee")
        node.payments = listOf(candidate.copy(id = "fee"))
        assertFalse(backgroundRecovery())
        assertEquals("", db.loadPendingSend()!!.paymentId)
        assertArchive(10.0, 11_000L)
        assertReplacementUnchanged()
    }

    private fun recordTradeFeePayment(paymentId: String) =
        db.writableDatabase.execSQL(
            "INSERT INTO trades (action, amount_usd, amount_btc, btc_price, trade_payment_id) VALUES ('buy', 1.0, 0.00001, 100000.0, '$paymentId')"
        )

    private fun ambiguousLostIdCandidate(): PaymentDetails {
        closeWithPendingStabilityAndReplace()
        db.setPendingSendPaymentId("")
        return node.payments
            .single()
            .copy(latestUpdateTimestamp = (System.currentTimeMillis() / 1000).toULong())
    }

    private fun reopenLegacyStabilityMarker(claimAgeSecs: Long = 0, amountMsat: Long = 1_000_000) {
        db.writableDatabase.execSQL("DROP TABLE pending_stability_send")
        db.writableDatabase.execSQL(
            "CREATE TABLE pending_stability_send (id INTEGER PRIMARY KEY CHECK (id = 1), payment_id TEXT NOT NULL, amount_msat INTEGER NOT NULL, price REAL NOT NULL, created_at INTEGER NOT NULL)"
        )
        db.writableDatabase.execSQL(
            "INSERT INTO pending_stability_send VALUES (1, 'send', $amountMsat, 100000.0, strftime('%s','now') - $claimAgeSecs)"
        )
        db.close()
        db = DatabaseService(context)
        field(state, "databaseService").set(state, db)
    }

    private fun prepareAccountedLegacyStability() {
        // The pre-upgrade writer commits history and the backing debit together. Simulate
        // stopping before the separate marker clear, then reopening the old schema on upgrade.
        val result =
            db.recordPaymentAndMaybeUpdateBacking(
                paymentId = "send",
                paymentType = "stability",
                direction = "sent",
                amountMsat = 1_000_000,
                amountUSD = 1.0,
                btcPrice = 100_000.0,
                userChannelId = "7",
                backingDeltaSats = -1_000,
            )
        assertEquals(10_000L, result.backingSats)
        reopenLegacyStabilityMarker()
        restart()
        setBooks(10.0, 10_000)
        node.channels = listOf(channel(receiver = 19_000))
        node.payments =
            listOf(
                terminal()
                    .copy(kind = PaymentKind.Spontaneous("hash", null), amountMsat = 1_000_000uL)
            )
        price(100_000.0)
        assertNull(db.loadPendingSend()!!.userChannelId)
        assertLegacyAccountingUnchanged()
    }

    private fun assertLegacyAccountingUnchanged() {
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(1, db.getRecentPayments().count { it.paymentId == "send" })
        assertEquals("completed", payment().status)
        assertEquals(1_000_000L, payment().amountMsat)
        assertNoStabilityOriginRecorded()
    }

    private fun assertNoStabilityOriginRecorded() {
        db.readableDatabase
            .rawQuery("SELECT count(*) FROM outgoing_stability_accounting", null)
            .use {
                assertTrue(it.moveToFirst())
                assertEquals(0, it.getInt(0))
            }
    }

    private fun assertNativeSpendAfterLegacyRecovery() {
        assertNull(db.loadPendingSend())
        // The settled channel has 19,000 sats, with 10,000 backing and 9,000 native.
        // A 1,000-sat send plus its capped fees fits entirely in native BTC.
        assertEquals(
            "native-after-upgrade",
            state.nodeService.sendTrackedLightningPayment("lightning", 1_000_000, null) {
                "native-after-upgrade"
            },
        )
    }

    /**
     * The pre-upgrade writer sent the payment but stopped before committing history and the debit.
     */
    private fun prepareUnaccountedLegacyStability(amountMsat: Long = 1_000_000) {
        reopenLegacyStabilityMarker(amountMsat = amountMsat)
        restart()
        setBooks(10.0, 11_000)
        node.channels = listOf(channel(receiver = 19_000)) // 20,000 sats before the 1,000-sat send
        node.payments =
            listOf(
                terminal()
                    .copy(
                        kind = PaymentKind.Spontaneous("hash", null),
                        amountMsat = amountMsat.toULong(),
                    )
            )
        price(100_000.0)
        assertNull(db.loadPendingSend()!!.userChannelId)
        assertLegacyBooksUntouched()
    }

    private fun assertLegacyBooksUntouched() {
        assertEquals(11_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(0, db.getRecentPayments().count { it.paymentId == "send" })
        assertNoStabilityOriginRecorded()
    }

    private fun assertLegacyDebitAppliedOnce() {
        assertNull(db.loadPendingSend())
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(10.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(1, db.getRecentPayments().count { it.paymentId == "send" })
        assertEquals("completed", payment().status)
        assertEquals(1_000_000L, payment().amountMsat)
        assertEquals("7", db.outgoingStabilityOrigin("send"))
    }

    private fun assertLegacyPaymentOnRecordWithoutOrigin() {
        assertEquals(1, db.getRecentPayments().count { it.paymentId == "send" })
        assertEquals("completed", payment().status)
        assertNoStabilityOriginRecorded()
    }

    private fun closeWithPendingStabilityAndReplace() {
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
        db.setPendingSendPaymentId("send")
        node.payments =
            listOf(
                terminal()
                    .copy(
                        kind = PaymentKind.Spontaneous("hash", null),
                        amountMsat = 1_000_000uL,
                        status = PaymentStatus.PENDING,
                    )
            )
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
            return StabilityProcessingService::class
                .java
                .getDeclaredMethod(
                    "reconcilePendingOutgoingPayment",
                    Node::class.java,
                    String::class.java,
                )
                .apply { isAccessible = true }
                .invoke(controller.get(), node, db.writableDatabase.path) as Boolean
        } catch (e: InvocationTargetException) {
            throw e.targetException
        } finally {
            controller.destroy()
        }
    }

    private fun pending() =
        db.recordPendingLightningPayment("send", "lightning", 9_000_000, 100_000.0, "7")

    private fun payment() = db.getRecentPayments().single { it.paymentId == "send" }

    private fun terminal() =
        PaymentDetails(
            "send",
            PaymentKind.Bolt11("hash", null, null, null),
            9_000_000uL,
            123uL,
            PaymentDirection.OUTBOUND,
            PaymentStatus.SUCCEEDED,
            0uL,
        )

    @Suppress("UNCHECKED_CAST")
    private fun restart() {
        state = AppState(context)
        field(state, "databaseService").set(state, db)
        node = TestNode()
        field(state.nodeService, "node").set(state.nodeService, node)
        (field(state.nodeService, "_isRunning").get(state.nodeService) as MutableStateFlow<Boolean>)
            .value = true
        setBooks(10.0, 11_000)
    }

    @Suppress("UNCHECKED_CAST")
    private fun setBooks(
        expected: Double,
        backing: Long,
        uid: String = "7",
        cid: String = "channel",
    ) {
        (field(state, "_stableChannel").get(state) as MutableStateFlow<StableChannel>).value =
            StableChannel(
                userChannelId = uid,
                channelId = cid,
                expectedUSD = USD(expected),
                backingSats = backing,
                stableReceiverBTC = Bitcoin(20_000),
                latestPrice = 100_000.0,
            )
    }

    @Suppress("UNCHECKED_CAST")
    private fun price(value: Double) {
        state.priceService.seedPrice(value)
        (field(state.priceService, "_lastUpdate").get(state.priceService) as MutableStateFlow<Date>)
            .value = Date()
    }

    private fun field(target: Any, name: String) =
        target.javaClass.getDeclaredField(name).apply { isAccessible = true }

    private fun call(name: String) = invoke(name, emptyArray(), emptyArray())

    private fun event(event: Event) =
        invoke("handleEvent", arrayOf(Event::class.java), arrayOf(event))

    private fun invoke(name: String, types: Array<Class<*>>, args: Array<Any?>) {
        try {
            AppState::class
                .java
                .getDeclaredMethod(name, *types)
                .apply { isAccessible = true }
                .invoke(state, *args)
        } catch (e: InvocationTargetException) {
            throw e.targetException
        }
    }

    private fun assertArchive(expected: Double, backing: Long) {
        db.readableDatabase
            .rawQuery(
                "SELECT expected_usd, stable_sats FROM closed_channel_books WHERE user_channel_id = '7'",
                null,
            )
            .use {
                assertTrue(it.moveToFirst())
                assertEquals(expected, it.getDouble(0), 0.0)
                assertEquals(backing, it.getLong(1))
            }
    }
}
