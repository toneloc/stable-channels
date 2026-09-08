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

/** Covers the guards in updatePaymentTxid() that keep concurrent/racing close-txid resolvers
 *  (CloseTxidResolver, the ChannelClosed event handler, and detectOnchainDeposit's close-payout
 *  path) from attaching the same txid to two rows or overwriting an already-resolved row. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OnchainReceiveTxidDatabaseServiceTest {
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
    fun txidAlreadyUsedByAnotherRowIsNotReassigned() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "row-a", paymentType = "onchain", direction = "received",
            amountMsat = 100_000, status = "completed", txid = "shared-tx"
        )
        val rowB = service.recordPayment(
            paymentId = "row-b", paymentType = "onchain", direction = "received",
            amountMsat = 100_000, status = "pending", txid = null
        )

        assertFalse(service.updatePaymentTxid("row-b", "shared-tx"))
        assertNull(payment(service, rowB).txid)
        service.close()
    }

    @Test
    fun alreadyResolvedRowIsNotOverwritten() {
        val service = DatabaseService(context)
        val rowId = service.recordPayment(
            paymentId = "row-a", paymentType = "onchain", direction = "received",
            amountMsat = 50_000, status = "pending", txid = "original-tx"
        )

        assertFalse(service.updatePaymentTxid("row-a", "different-tx"))
        assertEquals("original-tx", payment(service, rowId).txid)
        service.close()
    }

    @Test
    fun unclaimedTxidIsAssigned() {
        val service = DatabaseService(context)
        val rowId = service.recordPayment(
            paymentId = "row-a", paymentType = "onchain", direction = "received",
            amountMsat = 50_000, status = "pending", txid = null, address = "bc1qtracked"
        )

        assertTrue(service.updatePaymentTxid("row-a", "fresh-tx"))
        val updated = payment(service, rowId)
        assertEquals("fresh-tx", updated.txid)
        assertEquals("bc1qtracked", updated.address)
        service.close()
    }

    @Test
    fun txidAlreadyUsedByRowWithNullPaymentIdIsNotReassigned() {
        // Regression test: the duplicate-txid guard used to key its exclusion on `payment_id`,
        // and `payment_id != ?` evaluates to NULL/unknown for a NULL payment_id row — silently
        // defeating the NOT EXISTS check for exactly the rows most likely to need it (e.g. some
        // channel-close bookkeeping paths leave payment_id null). The guard is now keyed on the
        // row's own primary key (`id`), which is never null.
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = null, paymentType = "onchain", direction = "received",
            amountMsat = 75_000, status = "completed", txid = "claimed-tx"
        )
        val rowB = service.recordPayment(
            paymentId = "row-b", paymentType = "onchain", direction = "received",
            amountMsat = 75_000, status = "pending", txid = null
        )

        assertFalse(service.updatePaymentTxid("row-b", "claimed-tx"))
        assertNull(payment(service, rowB).txid)
        service.close()
    }

    private fun payment(service: DatabaseService, id: Long): PaymentRecord =
        service.getRecentPayments(100).single { it.id == id }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) assertTrue(file.delete()) }
        assertFalse(dbFile.exists())
    }
}
