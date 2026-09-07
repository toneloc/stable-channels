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
