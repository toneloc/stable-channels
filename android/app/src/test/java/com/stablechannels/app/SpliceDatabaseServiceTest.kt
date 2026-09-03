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
