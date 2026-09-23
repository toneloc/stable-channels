package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * Covers getConfirmationsForCloseTxid() (#316 follow-up): the Home card used to clear its "Channel
 * closing..." label based on the wallet's AGGREGATE spendable balance going positive, which can
 * already be true from other, unrelated funds while the close's own output is still confirming —
 * mislabeling a later, unrelated on-chain receive as still being the old close. This looks up THIS
 * close's own row by txid instead.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CloseTxidConfirmationDatabaseServiceTest {
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
    fun returnsConfirmationsForMatchingCloseTxid() {
        val service = DatabaseService(context)
        val rowId =
            service.recordPayment(
                paymentId = "close-1",
                paymentType = "channel_close",
                direction = "received",
                amountMsat = 500_000,
                status = "pending",
                txid = "close-tx",
            )
        service.updatePaymentConfirmationState(rowId, confirmations = 3, status = "pending")

        assertEquals(3, service.getConfirmationsForCloseTxid("close-tx"))
        service.close()
    }

    @Test
    fun unknownTxidReturnsNull() {
        val service = DatabaseService(context)
        assertNull(service.getConfirmationsForCloseTxid("never-seen-tx"))
        service.close()
    }

    @Test
    fun sameTxidOnADifferentPaymentTypeIsNotMatched() {
        // A close txid shouldn't accidentally match an unrelated onchain row that happens to
        // reuse the same txid string (defensive: type filter matters here, not just txid).
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain-1",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 500_000,
            status = "completed",
            txid = "shared-tx",
        )

        assertNull(service.getConfirmationsForCloseTxid("shared-tx"))
        service.close()
    }

    @Test
    fun matchesTheCorrectRowAmongMultipleCloses() {
        val service = DatabaseService(context)
        val olderClose =
            service.recordPayment(
                paymentId = "close-old",
                paymentType = "channel_close",
                direction = "received",
                amountMsat = 500_000,
                status = "pending",
                txid = "old-close-tx",
            )
        service.updatePaymentConfirmationState(olderClose, confirmations = 6, status = "completed")
        val newerClose =
            service.recordPayment(
                paymentId = "close-new",
                paymentType = "channel_close",
                direction = "received",
                amountMsat = 700_000,
                status = "pending",
                txid = "new-close-tx",
            )
        service.updatePaymentConfirmationState(newerClose, confirmations = 2, status = "pending")

        assertEquals(6, service.getConfirmationsForCloseTxid("old-close-tx"))
        assertEquals(2, service.getConfirmationsForCloseTxid("new-close-tx"))
        service.close()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
    }
}
