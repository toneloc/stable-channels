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
        assertEquals(emptyList<Any>(), service.findTxidlessReceives("tx1", 2_323_000, nowSecs()))
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
        assertEquals(emptyList<Any>(), service.findTxidlessReceives("tx9", 5_000_000, nowSecs()))
        val candidate = service.findTxidlessReceives("tx1", 2_323_000, nowSecs()).single()
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

        assertEquals(emptyList<Any>(), service.findTxidlessReceives("tx1", 2_323_000, nowSecs()))
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

        assertEquals(
            "addrA",
            service.findTxidlessReceives("tx1", 2_323_000, nowSecs()).single().address,
        )
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
        val id = service.findTxidlessReceives("tx1", 2_323_000, nowSecs()).single().id

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
    fun waitsWhileSpliceHasNoTxid() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "splice",
            paymentType = "splice_out",
            direction = "sent",
            amountMsat = 25_000_000,
            status = "pending",
        )

        assertEquals(-1L, service.recordMissedReceive("tx1", 5_991, null, null, "addr", since))
        service.updatePaymentTxid("splice", "splicetx")
        assertNotEquals(-1L, service.recordMissedReceive("tx1", 5_991, null, null, "addr", since))
        assertEquals(-1L, service.recordMissedReceive("splicetx", 5_991, null, null, "addr", since))
        assertEquals(listOf("tx1"), service.getPendingOnchainReceives().map { it.txid })
        service.close()
    }

    @Test
    fun waitsWhileSpliceInHasNoTxid() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "splice",
            paymentType = "splice_in",
            direction = "sent",
            amountMsat = 25_000_000,
            status = "pending",
        )

        assertEquals(-1L, service.recordMissedReceive("tx1", 5_991, null, null, "addr", since))
        service.close()
    }

    private fun nowSecs() = System.currentTimeMillis() / 1000

    private fun DatabaseService.recordPlaceholder(sats: Long, address: String = "addr"): Long =
        recordPayment(
            paymentId = "onchain_deposit_$sats",
            paymentType = "onchain",
            direction = "received",
            amountMsat = sats * 1000,
            status = "pending",
            address = address,
        )

    private fun DatabaseService.pendingReceives() =
        getPendingOnchainReceives().map { it.txid to it.amountMsat / 1000 }.toSet()

    @Test
    fun mergedPlaceholderIsReplacedByOneRowPerTxid() {
        val service = DatabaseService(context)
        service.recordPlaceholder(29_967)

        service.recordMissedReceive("txA", 15_635, null, null, "addr", since)
        assertEquals(setOf(null to 29_967L, "txA" to 15_635L), service.pendingReceives())

        service.recordWebSocketReceive(
            "onchain_receive_txB",
            14_332_000,
            null,
            null,
            "txB",
            "addr",
        )
        assertEquals(setOf("txA" to 15_635L, "txB" to 14_332L), service.pendingReceives())
        service.close()
    }

    @Test
    fun keepsPlaceholderWhenLaterRowsDoNotAddUpToIt() {
        val service = DatabaseService(context)
        service.recordPlaceholder(50_000)

        service.recordMissedReceive("tx1", 20_000, null, null, "addr", since)
        service.recordMissedReceive("tx2", 20_000, null, null, "addr", since)
        service.recordMissedReceive("tx3", 50_000, null, null, "other", since)

        assertEquals(
            setOf(null to 50_000L, "tx1" to 20_000L, "tx2" to 20_000L, "tx3" to 50_000L),
            service.pendingReceives(),
        )
        service.close()
    }

    @Test
    fun mergedPlaceholderIsReplacedWhenALaterDepositIsRecordedToo() {
        val service = DatabaseService(context)
        service.recordPlaceholder(29_967)
        service.recordPlaceholder(10_000)

        service.recordMissedReceive("txC", 10_000, null, null, "addr", since)
        service.recordMissedReceive("txB", 14_332, null, null, "addr", since)
        service.recordMissedReceive("txA", 15_635, null, null, "addr", since)

        assertEquals(
            setOf("txA" to 15_635L, "txB" to 14_332L, "txC" to 10_000L),
            service.pendingReceives(),
        )
        service.close()
    }

    @Test
    fun failedRowsDoNotCountTowardMergedPlaceholder() {
        val service = DatabaseService(context)
        service.recordPlaceholder(29_967)

        service.recordMissedReceive("txA", 15_635, null, null, "addr", since)
        service.failPaymentByTxid("txA")
        service.recordMissedReceive("txB", 14_332, null, null, "addr", since)

        assertEquals(setOf(null to 29_967L, "txB" to 14_332L), service.pendingReceives())
        service.close()
    }

    @Test
    fun depositsRecordedLongAfterPlaceholderDoNotCountTowardIt() {
        val service = DatabaseService(context)
        service.recordPlaceholder(30_000)
        service.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - ?",
            arrayOf(DatabaseService.MERGED_RECEIVE_WINDOW_SECS + 60),
        )

        service.recordMissedReceive("tx1", 10_000, null, null, "addr", since)
        service.recordMissedReceive("tx2", 20_000, null, null, "addr", since)

        assertEquals(
            setOf(null to 30_000L, "tx1" to 10_000L, "tx2" to 20_000L),
            service.pendingReceives(),
        )
        service.close()
    }

    @Test
    fun singleLaterRowWithSameAmountDoesNotDropPlaceholder() {
        val service = DatabaseService(context)
        service.recordPlaceholder(10_000)
        service.recordPayment(
            paymentId = "onchain_receive_txS",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 10_000_000,
            status = "pending",
            txid = "txS",
            address = "addr",
        )

        service.recordMissedReceive("tx2", 3_000, null, null, "addr", since)

        assertEquals(
            setOf(null to 10_000L, "txS" to 10_000L, "tx2" to 3_000L),
            service.pendingReceives(),
        )
        service.close()
    }

    @Test
    fun rowsRecordedBeforePlaceholderDoNotCountTowardIt() {
        val service = DatabaseService(context)
        service.recordMissedReceive("tx1", 10_000, null, null, "addr", since)
        service.writableDatabase.execSQL("UPDATE payments SET created_at = created_at - 60")
        service.recordPlaceholder(30_000)

        service.recordMissedReceive("tx2", 20_000, null, null, "addr", since)

        assertEquals(
            setOf("tx1" to 10_000L, null to 30_000L, "tx2" to 20_000L),
            service.pendingReceives(),
        )
        service.close()
    }

    @Test
    fun resolvedTxidOnlyLabelsPlaceholderWithItsAmount() {
        val service = DatabaseService(context)
        service.recordPlaceholder(29_967)

        assertEquals(false, service.reconcileResolvedReceiveTxid("txA", "addr", 15_635_000))
        assertEquals(setOf(null to 29_967L), service.pendingReceives())

        assertTrue(service.reconcileResolvedReceiveTxid("txAB", "addr", 29_967_000))
        assertEquals(setOf("txAB" to 29_967L), service.pendingReceives())
        service.close()
    }

    @Test
    fun rejectsTxidWalletLastUpdatedLongBeforePlaceholderWasCreated() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
            address = "addr",
        )
        val staleSeenAt = nowSecs() - DatabaseService.RECEIVE_ADOPTION_SLACK_SECS - 3600

        assertEquals(
            emptyList<Any>(),
            service.findTxidlessReceives("oldTx", 2_323_000, staleSeenAt),
        )
        assertEquals(1, service.findTxidlessReceives("newTx", 2_323_000, nowSecs()).size)
        service.close()
    }

    @Test
    fun acceptsTxidSeenJustBeforePlaceholderWasCreated() {
        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "onchain_deposit_x",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 2_323_000,
            status = "pending",
            address = "addr",
        )
        val seenAt = nowSecs() - DatabaseService.RECEIVE_ADOPTION_SLACK_SECS + 60

        assertEquals(1, service.findTxidlessReceives("tx1", 2_323_000, seenAt).size)
        service.close()
    }

    private fun DatabaseService.insertOnchain(
        id: String,
        direction: String = "sent",
        status: String = "pending",
        txid: String? = "tx_$id",
        ageDays: Long = 0,
        confirmations: Int = 0,
    ): Long {
        val rowId =
            recordPayment(
                paymentId = id,
                paymentType = "onchain",
                direction = direction,
                amountMsat = 1_000_000,
                status = status,
                txid = txid,
            )
        writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - ?, confirmations = ? WHERE id = ?",
            arrayOf(ageDays * 86400L, confirmations, rowId),
        )
        return rowId
    }

    private fun DatabaseService.statusOf(rowId: Long): String =
        readableDatabase
            .rawQuery("SELECT status FROM payments WHERE id = ?", arrayOf(rowId.toString()))
            .use {
                it.moveToFirst()
                it.getString(0)
            }

    @Test
    fun failsOnlyOldTxidlessPendingOnchainRows() {
        val service = DatabaseService(context)
        val oldTxidless =
            service.insertOnchain("a", direction = "received", txid = null, ageDays = 20)
        val oldWithTxid = service.insertOnchain("b", ageDays = 20)
        val recentTxidless = service.insertOnchain("c", direction = "received", txid = null)

        assertEquals(1, service.failStaleTxidlessOnchain())

        assertEquals("failed", service.statusOf(oldTxidless))
        assertEquals("pending", service.statusOf(oldWithTxid))
        assertEquals("pending", service.statusOf(recentTxidless))
        service.close()
    }

    @Test
    fun failsStaleRowOnlyWhenOldPendingAndUnconfirmed() {
        val service = DatabaseService(context)
        val old = service.insertOnchain("a", ageDays = 20)
        val oldConfirming = service.insertOnchain("b", ageDays = 20, confirmations = 2)
        val oldCompleted = service.insertOnchain("c", ageDays = 20, status = "completed")
        val recent = service.insertOnchain("d")

        assertEquals(false, service.failStaleOnchainRow(oldConfirming))
        assertEquals(false, service.failStaleOnchainRow(oldCompleted))
        assertEquals(false, service.failStaleOnchainRow(recent))
        assertTrue(service.failStaleOnchainRow(old))

        assertEquals("failed", service.statusOf(old))
        assertEquals("pending", service.statusOf(oldConfirming))
        assertEquals("completed", service.statusOf(oldCompleted))
        assertEquals("pending", service.statusOf(recent))
        service.close()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
    }
}
