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

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
    }
}
