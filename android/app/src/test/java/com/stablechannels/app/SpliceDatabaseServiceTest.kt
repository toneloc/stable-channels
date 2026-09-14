package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SpliceDatabaseServiceTest {
    private lateinit var context: Context
    private lateinit var dbFile: File

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        deleteDatabaseFiles()
    }

    @After
    fun tearDown() {
        deleteDatabaseFiles()
    }

    @Test
    fun negotiatedTxidUpdatesExactPendingRowAndReplayIsIdempotent() {
        val service = DatabaseService(context)
        val targetId = recordSplice(service, "splice_out")
        val otherId = recordSplice(service, "splice_in")

        assertEquals(targetId, service.assignPendingSpliceTxid("tx-target", targetId))
        assertEquals("tx-target", payment(service, targetId).txid)
        assertNull(payment(service, otherId).txid)
        assertEquals(targetId, service.assignPendingSpliceTxid("tx-target", targetId))
        service.close()
    }

    @Test
    fun restartRecoveryRequiresExactlyOneRecentPendingRow() {
        val service = DatabaseService(context)
        val firstId = recordSplice(service, "splice_out")
        val secondId = recordSplice(service, "splice_in")

        assertNull(service.assignPendingSpliceTxid("ambiguous-tx"))
        assertNull(payment(service, firstId).txid)
        assertNull(payment(service, secondId).txid)

        assertTrue(service.failPendingSplice(secondId))
        assertEquals(firstId, service.assignPendingSpliceTxid("single-tx"))
        assertEquals("single-tx", payment(service, firstId).txid)
        service.close()
    }

    @Test
    fun failedSpliceCannotBeReassignedOrCompleted() {
        val service = DatabaseService(context)
        val failedId = recordSplice(service, "splice_out", status = "failed")

        assertNull(service.assignPendingSpliceTxid("new-tx", failedId))
        service.writableDatabase.execSQL(
            "UPDATE payments SET txid = ? WHERE id = ?",
            arrayOf<Any>("failed-tx", failedId)
        )
        assertFalse(service.completeSplice("failed-tx"))
        assertEquals("failed", payment(service, failedId).status)
        service.close()
    }

    @Test
    fun expiredPendingSpliceIsNotRecoveredOrFailedByAnEvent() {
        val service = DatabaseService(context)
        val now = 2_000_000L
        val expiredId = recordSplice(service, "splice_out")
        service.writableDatabase.execSQL(
            "UPDATE payments SET created_at = ? WHERE id = ?",
            arrayOf<Any>(
                now - DatabaseService.PENDING_SPLICE_WITHOUT_TXID_TIMEOUT_SECS - 1,
                expiredId
            )
        )

        assertNull(service.assignPendingSpliceTxid("late-tx", expiredId, now))
        assertFalse(service.failPendingSplice(expiredId, now))
        assertEquals("pending", payment(service, expiredId).status)
        assertNull(payment(service, expiredId).txid)
        service.close()
    }

    @Test
    fun txidAlreadyUsedByAnotherPaymentIsNotReassigned() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "existing-payment",
            paymentType = "onchain",
            direction = "sent",
            amountMsat = 1_000,
            txid = "used-tx"
        )
        val pendingId = recordSplice(service, "splice_out")

        assertNull(service.assignPendingSpliceTxid("used-tx", pendingId))
        assertNull(payment(service, pendingId).txid)
        service.close()
    }

    // #316 follow-up (GPT-5.6-sol review): a splice-out paying our own currently tracked
    // receive address is a self-send — the unconditional websocket "Receive" handler (added to
    // stop deposits landing mid-splice from being silently dropped) can record it as a plain
    // onchain/received row before this splice's own txid is known, using the exact same txid
    // the splice later gets confirmed with. Both callers of assignPendingSpliceTxid() always
    // pass the splice's OWN observed txid, so a collision here can only ever mean "this row and
    // my splice describe the same transaction" — never a coincidentally-different deposit
    // (which would necessarily have a different txid and never reach this collision at all).
    @Test
    fun selfSendOnchainReceivedRowSharingTheSpliceTxidIsReconciledAndReplaced() {
        val service = DatabaseService(context)
        val duplicateRowId = service.recordPayment(
            paymentId = "onchain_receive_self-send-tx", paymentType = "onchain",
            direction = "received", amountMsat = 50_000, status = "pending",
            txid = "self-send-tx", address = "bc1qourtrackedaddress"
        )
        val spliceId = recordSplice(service, "splice_out")

        assertEquals(spliceId, service.assignPendingSpliceTxid("self-send-tx", spliceId))
        assertEquals("self-send-tx", payment(service, spliceId).txid)
        // The duplicate onchain row must be gone entirely, not just left orphaned — otherwise
        // it would still show up in History/Home as a second, phantom pending deposit.
        assertTrue(service.getRecentPayments(100).none { it.id == duplicateRowId })
        service.close()
    }

    // Astra review finding: after assignPendingSpliceTxid() reconciles a self-send (deleting the
    // websocket-recorded receive row and giving the txid to the splice row), the balance-delta
    // detector's frozen baseline never advanced during the splice — so once the splice confirms
    // and the detector unfreezes, it sees the same balance increase again and must NOT re-insert
    // a new receive row for a txid that's already accounted for on the splice row. This mirrors
    // the check detectOnchainDeposit() now performs via paymentExistsForTxid() before inserting.
    @Test
    fun paymentExistsForTxidPreventsDetectorFromRecreatingAReconciledSelfSend() {
        val service = DatabaseService(context)
        service.recordWebSocketReceive(
            paymentId = "onchain_receive_self-send-tx", amountMsat = 50_000_000,
            amountUSD = null, btcPrice = null, txid = "self-send-tx", address = "bc1qourtrackedaddress"
        )
        val spliceId = recordSplice(service, "splice_out")
        assertEquals(spliceId, service.assignPendingSpliceTxid("self-send-tx", spliceId))

        // Detector wakes up post-confirmation, resolves the same txid via the tracked address,
        // and must see it as already claimed rather than inserting a second row.
        assertTrue(service.paymentExistsForTxid("self-send-tx"))
        assertEquals(
            1,
            service.getRecentPayments(100).count { it.txid == "self-send-tx" }
        )
        service.close()
    }

    // #316 review follow-up: completeConfirmedSplice() queries this to decide whether to advance
    // the deposit detector's baseline (a self-send splice-out to an untracked address otherwise
    // becomes a permanent, unresolvable phantom pending receive — see
    // SpliceOutBaselineAdvanceDecisionTest for the baseline arithmetic itself).
    @Test
    fun getPaymentTypeDirectionAmountMsatReturnsTheRowsFieldsById() {
        val service = DatabaseService(context)
        val spliceOutId = service.recordPayment(
            paymentId = null, paymentType = "splice_out", direction = "sent",
            amountMsat = 10_000, address = "bc1qourownaddress"
        )

        val row = service.getPaymentTypeDirectionAmountMsat(spliceOutId)

        assertEquals("splice_out", row?.paymentType)
        assertEquals("sent", row?.direction)
        assertEquals(10_000L, row?.amountMsat)
        assertEquals("bc1qourownaddress", row?.address)
        service.close()
    }

    @Test
    fun getPaymentTypeDirectionAmountMsatReturnsNullForAnUnknownId() {
        val service = DatabaseService(context)
        assertNull(service.getPaymentTypeDirectionAmountMsat(999_999L))
        service.close()
    }

    // #316 review round 3: gates the baseline advance to genuine self-sends — an external splice-
    // out destination never raises our own balance, so advancing for one would misattribute a
    // concurrent, unrelated deposit's sats into the baseline instead of surfacing them.
    @Test
    fun isKnownReceiveAddressIsTrueOnceWeHaveReceivedThere() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_receive_older-tx", paymentType = "onchain", direction = "received",
            amountMsat = 20_000, status = "completed", txid = "older-tx", address = "bc1qourownaddress"
        )

        assertTrue(service.isKnownReceiveAddress("bc1qourownaddress"))
        service.close()
    }

    @Test
    fun isKnownReceiveAddressIsFalseForAnAddressWeveNeverReceivedTo() {
        val service = DatabaseService(context)
        assertFalse(service.isKnownReceiveAddress("bc1qsomeoneelsesaddress"))
        service.close()
    }

    @Test
    fun completedOnchainReceivedRowSharingTheSpliceTxidIsAlsoReconciled() {
        // The duplicate can already be 'completed' (6+ confirmations) by the time the splice's
        // own 1-conf threshold triggers this assignment — status must not matter, only that it's
        // a plain onchain/received row.
        val service = DatabaseService(context)
        val duplicateRowId = service.recordPayment(
            paymentId = "onchain_receive_self-send-tx", paymentType = "onchain",
            direction = "received", amountMsat = 50_000, status = "completed",
            txid = "self-send-tx"
        )
        val spliceId = recordSplice(service, "splice_out")

        assertEquals(spliceId, service.assignPendingSpliceTxid("self-send-tx", spliceId))
        assertEquals("self-send-tx", payment(service, spliceId).txid)
        assertTrue(service.getRecentPayments(100).none { it.id == duplicateRowId })
        service.close()
    }

    @Test
    fun sentOnchainRowSharingATxidIsNotTreatedAsASelfSendDuplicate() {
        // Regression guard for the reconciliation itself: only an onchain/RECEIVED row is a
        // plausible self-send duplicate. An onchain/SENT row sharing a txid is a different kind
        // of conflict entirely and must still block assignment rather than being deleted.
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "sent-row", paymentType = "onchain", direction = "sent",
            amountMsat = 50_000, status = "completed", txid = "shared-tx"
        )
        val spliceId = recordSplice(service, "splice_out")

        assertNull(service.assignPendingSpliceTxid("shared-tx", spliceId))
        assertNull(payment(service, spliceId).txid)
        service.close()
    }

    // GPT-5.6-sol review (round 2): the reconciling delete must never commit unless the txid
    // assignment itself actually succeeds — otherwise a legitimate onchain/received row can be
    // destroyed for an assignment that fails anyway (ambiguous or expired candidate), losing the
    // user's deposit record for nothing. These two tests pin that the conflicting row survives
    // fully intact whenever the candidate resolution fails, in the two ways it can fail.

    @Test
    fun ambiguousSpliceCandidateLeavesConflictingReceiveRowIntact() {
        val service = DatabaseService(context)
        val duplicateRowId = service.recordPayment(
            paymentId = "onchain_receive_self-send-tx", paymentType = "onchain",
            direction = "received", amountMsat = 50_000, status = "pending",
            txid = "self-send-tx"
        )
        recordSplice(service, "splice_out")
        recordSplice(service, "splice_out")

        // No paymentRowId given: two untouched pending splice_out rows makes the candidate
        // ambiguous, so assignment must refuse — and must not have deleted anything either.
        assertNull(service.assignPendingSpliceTxid("self-send-tx"))
        assertEquals("self-send-tx", payment(service, duplicateRowId).txid)
        assertTrue(service.getRecentPayments(100).any { it.id == duplicateRowId })
        service.close()
    }

    @Test
    fun expiredSpliceCandidateLeavesConflictingReceiveRowIntact() {
        val service = DatabaseService(context)
        val now = 2_000_000L
        val duplicateRowId = service.recordPayment(
            paymentId = "onchain_receive_self-send-tx", paymentType = "onchain",
            direction = "received", amountMsat = 50_000, status = "pending",
            txid = "self-send-tx"
        )
        val spliceId = recordSplice(service, "splice_out")
        service.writableDatabase.execSQL(
            "UPDATE payments SET created_at = ? WHERE id = ?",
            arrayOf<Any>(
                now - DatabaseService.PENDING_SPLICE_WITHOUT_TXID_TIMEOUT_SECS - 1,
                spliceId
            )
        )

        assertNull(service.assignPendingSpliceTxid("self-send-tx", spliceId, now))
        assertEquals("self-send-tx", payment(service, duplicateRowId).txid)
        assertTrue(service.getRecentPayments(100).any { it.id == duplicateRowId })
        assertNull(payment(service, spliceId).txid)
        service.close()
    }

    @Test
    fun failingByIdChangesOnlyThatPendingSplice() {
        val service = DatabaseService(context)
        val targetId = recordSplice(service, "splice_out")
        val otherId = recordSplice(service, "splice_in")

        assertTrue(service.failPendingSplice(targetId))
        assertEquals("failed", payment(service, targetId).status)
        assertEquals("pending", payment(service, otherId).status)
        service.close()
    }

    // These model the exact race both reviewers flagged for the AppState generation guard: an
    // old splice's stale failure/confirmation handler captures its row id before an async check,
    // a genuinely new splice starts and creates its own row in the meantime, and only then does
    // the stale handler act. AppState itself can't be unit-instantiated (it requires a live node
    // + Android Application context), but its safety depends entirely on DatabaseService methods
    // being keyed by the captured id and never touching a different row — which these tests prove
    // directly for both splice directions.

    @Test
    fun staleFailureAfterNewSpliceOutStartedOnlyFailsOldRow() {
        val service = DatabaseService(context)
        val oldId = recordSplice(service, "splice_out")
        // Old splice's failure handler would have captured oldId here, before an async check.
        // A genuinely new splice then starts and creates its own row while that check is in
        // flight (its captured monitor generation prevents this from happening, but the DB call
        // itself must be safe regardless).
        val newId = recordSplice(service, "splice_out")

        // Stale handler for the OLD splice finally resolves and fails using its captured id.
        assertTrue(service.failPendingSplice(oldId))
        assertEquals("failed", payment(service, oldId).status)
        assertEquals("pending", payment(service, newId).status)
        service.close()
    }

    @Test
    fun staleFailureAfterNewSpliceInStartedOnlyFailsOldRow() {
        val service = DatabaseService(context)
        val oldId = recordSplice(service, "splice_in")
        val newId = recordSplice(service, "splice_in")

        assertTrue(service.failPendingSplice(oldId))
        assertEquals("failed", payment(service, oldId).status)
        assertEquals("pending", payment(service, newId).status)
        service.close()
    }

    @Test
    fun staleConfirmationWithCapturedNullRowIdRefusesToBindWhenAmbiguous() {
        val service = DatabaseService(context)
        // Models a monitor resumed after a process restart (pendingSplice was never
        // reconstructed, so its captured paymentRowId is null) whose confirmation resolves after
        // a second, genuinely new pending splice has also been created — assignPendingSpliceTxid
        // must refuse to guess between them rather than binding the confirmed txid to the wrong row.
        val oldId = recordSplice(service, "splice_out")
        val newId = recordSplice(service, "splice_out")

        assertNull(service.assignPendingSpliceTxid("resumed-tx", null))
        assertNull(payment(service, oldId).txid)
        assertNull(payment(service, newId).txid)
        service.close()
    }

    @Test
    fun staleConfirmationWithCapturedRowIdBindsOnlyThatRowEvenIfNewerRowExists() {
        val service = DatabaseService(context)
        // Unlike the null-capture case above, a monitor that captured a concrete row id at
        // launch must always be able to finalize that exact row, regardless of any newer splice
        // created afterward.
        val oldId = recordSplice(service, "splice_out")
        val newId = recordSplice(service, "splice_out")

        assertEquals(oldId, service.assignPendingSpliceTxid("late-confirm-tx", oldId))
        assertEquals("late-confirm-tx", payment(service, oldId).txid)
        assertNull(payment(service, newId).txid)
        service.close()
    }

    private fun recordSplice(
        service: DatabaseService,
        type: String,
        status: String = "pending"
    ): Long = service.recordPayment(
        paymentId = null,
        paymentType = type,
        direction = if (type == "splice_out") "sent" else "received",
        amountMsat = 10_000,
        status = status
    )

    private fun payment(service: DatabaseService, id: Long): PaymentRecord =
        service.getRecentPayments(100).single { it.id == id }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) assertTrue(file.delete()) }
        assertFalse(dbFile.exists())
    }
}
