package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** Home shows an onchain send as a "-" row until it confirms (#380). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OnchainSendPendingListDatabaseServiceTest {
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
    fun pendingSendsAreReturnedOldestFirstAndExcludeReceives() {
        val service = DatabaseService(context)
        val first = record(service, "send-1", "sent", "pending")
        val second = record(service, "send-2", "sent", "pending")
        record(service, "receive-1", "received", "pending")
        setCreatedAt(service, first, 1_000L)
        setCreatedAt(service, second, 2_000L)

        assertEquals(listOf(first, second), service.getPendingOnchainSends().map { it.id })
        assertEquals(listOf("receive-1"), service.getPendingOnchainReceives().map { it.paymentId })
        service.close()
    }

    @Test
    fun confirmedFailedAndNonOnchainSendsAreExcluded() {
        val service = DatabaseService(context)
        record(service, "done", "sent", "completed")
        record(service, "failed", "sent", "failed")
        service.recordPayment(
            paymentId = "splice",
            paymentType = "splice_out",
            direction = "sent",
            amountMsat = 100_000,
            status = "pending",
        )
        val pending = record(service, "pending", "sent", "pending")

        assertEquals(listOf(pending), service.getPendingOnchainSends().map { it.id })
        service.close()
    }

    @Test
    fun confirmedSendLeavesTheList() {
        val service = DatabaseService(context)
        val id = record(service, "send", "sent", "pending")
        assertEquals(1, service.getPendingOnchainSends().size)

        service.updatePaymentConfirmationState(id, confirmations = 6, status = "completed")

        assertTrue(service.getPendingOnchainSends().isEmpty())
        service.close()
    }

    private fun record(
        service: DatabaseService,
        paymentId: String,
        direction: String,
        status: String,
    ): Long =
        service.recordPayment(
            paymentId = paymentId,
            paymentType = "onchain",
            direction = direction,
            amountMsat = 100_000,
            status = status,
            txid = paymentId,
        )

    private fun setCreatedAt(service: DatabaseService, rowId: Long, createdAt: Long) {
        service.writableDatabase.execSQL(
            "UPDATE payments SET created_at = ? WHERE id = ?",
            arrayOf<Any>(createdAt, rowId),
        )
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
    }
}
