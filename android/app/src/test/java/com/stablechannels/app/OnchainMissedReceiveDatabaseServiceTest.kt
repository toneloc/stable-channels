package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OnchainMissedReceiveDatabaseServiceTest {
    private lateinit var context: Context
    private lateinit var dbFile: File
    private val since = 0L

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
    fun recordsMissingDepositOnce() {
        val service = DatabaseService(context)

        val first = service.recordMissedReceive("tx1", 11_732, null, null, "addr", since)
        val second = service.recordMissedReceive("tx1", 11_732, null, null, "addr", since)

        assertNotEquals(-1L, first)
        assertEquals(-1L, second)
        assertEquals(listOf("tx1"), service.getPendingOnchainReceives().map { it.txid })
        service.close()
    }

    @Test
    fun skipsWhenResolvedRowWithSameAmountExists() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "old",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 11_732_000,
            status = "completed",
        )

        assertEquals(-1L, service.recordMissedReceive("tx1", 11_732, null, null, "addr", since))
        service.close()
    }

    @Test
    fun adoptsTxidlessPendingPlaceholderInsteadOfDuplicating() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 11_732_000,
            status = "pending",
            address = "addr",
        )

        service.recordMissedReceive("tx1", 11_732, null, null, "addr", since)

        assertEquals(listOf("tx1"), service.getPendingOnchainReceives().map { it.txid })
        service.close()
    }

    @Test
    fun addresslessPlaceholderIsNeverACandidateForTxidAdoption() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
        )

        assertEquals(false, service.hasTxidlessPendingReceive())
        assertEquals(emptyList<Any>(), service.findTxidlessReceives("tx1", 2_323_000))
        service.close()
    }

    @Test
    fun adoptsTxidForAddressedPlaceholderWithMatchingAmount() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
            address = "addrA",
        )

        assertTrue(service.hasTxidlessPendingReceive())
        assertEquals(emptyList<Any>(), service.findTxidlessReceives("tx9", 5_000_000))
        val candidate = service.findTxidlessReceives("tx1", 2_323_000).single()
        assertEquals("addrA", candidate.address)
        assertTrue(service.adoptTxidForRow(candidate.id, "tx1"))

        assertEquals(listOf("tx1"), service.getPendingOnchainReceives().map { it.txid })
        assertEquals(false, service.hasTxidlessPendingReceive())
        service.close()
    }

    @Test
    fun adoptSkipsTxidAlreadyOnARow() {
        val service = DatabaseService(context)
        service.recordMissedReceive("tx1", 2_323, null, null, "addr", since)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
            address = "addr",
        )

        assertEquals(emptyList<Any>(), service.findTxidlessReceives("tx1", 2_323_000))
        service.close()
    }

    @Test
    fun findReturnsAddressSoCallerCanRejectMismatch() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
            address = "addrA",
        )

        assertEquals("addrA", service.findTxidlessReceives("tx1", 2_323_000).single().address)
        service.close()
    }

    @Test
    fun adoptDoesNotOverwriteATxidSetMeanwhile() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
            address = "addr",
        )
        val id = service.findTxidlessReceives("tx1", 2_323_000).single().id

        assertTrue(service.adoptTxidForRow(id, "tx1"))
        assertEquals(false, service.adoptTxidForRow(id, "tx2"))
        assertEquals(listOf("tx1"), service.getPendingOnchainReceives().map { it.txid })
        service.close()
    }

    @Test
    fun recordsDepositWhenTxidlessRowOfSameAmountFailed() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "failed_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 11_732_000,
            status = "failed",
        )

        assertNotEquals(-1L, service.recordMissedReceive("tx2", 11_732, null, null, "addr", since))
        service.close()
    }

    @Test
    fun recordsReplacementWhenSameAmountRowFailed() {
        val service = DatabaseService(context)
        service.recordMissedReceive("tx1", 11_732, null, null, "addr", since)
        service.failPaymentByTxid("tx1")

        assertNotEquals(-1L, service.recordMissedReceive("tx2", 11_732, null, null, "addr", since))
        assertEquals(listOf("tx2"), service.getPendingOnchainReceives().map { it.txid })
        service.close()
    }

    @Test
    fun recordsSecondDepositOfSameAmountWithDifferentTxid() {
        val service = DatabaseService(context)
        service.recordMissedReceive("tx1", 11_732, null, null, "addr", since)

        assertNotEquals(-1L, service.recordMissedReceive("tx2", 11_732, null, null, "addr", since))
        assertEquals(
            setOf("tx1", "tx2"),
            service.getPendingOnchainReceives().map { it.txid }.toSet(),
        )
        service.close()
    }

    @Test
    fun failsOnlyOldUnconfirmedPendingOnchainRows() {
        val service = DatabaseService(context)
        fun insert(id: String, status: String = "pending", direction: String = "sent") =
            service.recordPayment(
                paymentId = id,
                paymentType = "onchain",
                direction = direction,
                amountMsat = 1_000_000,
                status = status,
                txid = "tx_$id",
            )
        val old = insert("old")
        val oldReceive = insert("oldReceive", direction = "received")
        val oldConfirming = insert("oldConfirming")
        val oldCompleted = insert("oldCompleted", status = "completed")
        val recent = insert("recent")
        val db = service.writableDatabase
        val twentyDays = 20 * 86400L
        listOf(old, oldReceive, oldConfirming, oldCompleted).forEach {
            db.execSQL("UPDATE payments SET created_at = created_at - $twentyDays WHERE id = $it")
        }
        db.execSQL("UPDATE payments SET confirmations = 2 WHERE id = $oldConfirming")

        assertEquals(2, service.failStalePendingOnchain())

        fun statusOf(id: Long) =
            db.rawQuery("SELECT status FROM payments WHERE id = ?", arrayOf(id.toString())).use {
                it.moveToFirst()
                it.getString(0)
            }
        assertEquals("failed", statusOf(old))
        assertEquals("failed", statusOf(oldReceive))
        assertEquals("pending", statusOf(oldConfirming))
        assertEquals("completed", statusOf(oldCompleted))
        assertEquals("pending", statusOf(recent))
        assertEquals(
            setOf("tx_recent", "tx_oldConfirming"),
            service.getPendingOnchainSends().map { it.txid }.toSet(),
        )
        service.close()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
    }
}
