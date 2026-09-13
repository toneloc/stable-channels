package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File

/** Covers getPendingOnchainReceives() (#316 follow-up): a deposit can arrive while another is
 *  still confirming, or while a splice/close is in flight, so the Home card must be able to show
 *  more than one pending row. The query used to select the OLDEST rows first (ORDER BY
 *  created_at ASC LIMIT n) — a handful of stuck/never-confirming rows could then starve a
 *  genuinely new deposit out of the list entirely, hiding it from the user. It now selects the
 *  NEWEST rows first internally, then re-sorts that selection back to oldest-first for display. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OnchainReceivePendingListDatabaseServiceTest {
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
    fun noPendingReceivesReturnsEmptyList() {
        val service = DatabaseService(context)
        assertTrue(service.getPendingOnchainReceives().isEmpty())
        service.close()
    }

    @Test
    fun multiplePendingReceivesAreReturnedOldestFirst() {
        val service = DatabaseService(context)
        val firstId = recordPendingReceive(service, "row-1")
        setCreatedAt(service, firstId, 1_000L)
        val secondId = recordPendingReceive(service, "row-2")
        setCreatedAt(service, secondId, 2_000L)
        val thirdId = recordPendingReceive(service, "row-3")
        setCreatedAt(service, thirdId, 3_000L)

        val rows = service.getPendingOnchainReceives()
        assertEquals(listOf(firstId, secondId, thirdId), rows.map { it.id })
        service.close()
    }

    @Test
    fun newDepositIsNotStarvedOutByOlderStuckRows() {
        // Regression: with a low limit and ORDER BY created_at ASC, a pile of old
        // never-confirming rows could fill the whole result, pushing a brand-new deposit out
        // of the list entirely.
        val service = DatabaseService(context)
        val limit = 3
        val stuckIds = (1..5).map { i ->
            val id = recordPendingReceive(service, "stuck-$i")
            setCreatedAt(service, id, i.toLong() * 1_000)
            id
        }
        val freshId = recordPendingReceive(service, "fresh")
        setCreatedAt(service, freshId, 999_999L)

        val rows = service.getPendingOnchainReceives(limit = limit)
        assertEquals(limit, rows.size)
        assertTrue("newest deposit must survive the limit", rows.any { it.id == freshId })
        assertEquals(freshId, rows.last().id) // still oldest-first, so newest sorts last
        assertTrue(rows.none { it.id == stuckIds.first() }) // the very oldest stuck row is dropped
        service.close()
    }

    @Test
    fun nonPendingOrWrongTypeRowsAreExcluded() {
        val service = DatabaseService(context)
        recordPendingReceive(service, "pending-one")
        service.recordPayment(
            paymentId = "completed-onchain", paymentType = "onchain", direction = "received",
            amountMsat = 50_000, status = "completed"
        )
        service.recordPayment(
            paymentId = "pending-splice", paymentType = "splice_in", direction = "received",
            amountMsat = 50_000, status = "pending"
        )
        service.recordPayment(
            paymentId = "pending-sent", paymentType = "onchain", direction = "sent",
            amountMsat = 50_000, status = "pending"
        )

        val rows = service.getPendingOnchainReceives()
        assertEquals(1, rows.size)
        assertEquals("pending-one", rows.single().paymentId)
        service.close()
    }

    @Test
    fun sameSecondTimestampTiesAreBrokenDeterministicallyById() {
        // #316 follow-up (GPT-5.6-sol review): created_at has only 1-second resolution, so rows
        // inserted within the same second are tied without a secondary sort key. The newest-N
        // selection and the oldest-first display must both fall back to id to keep the boundary
        // deterministic, rather than depending on unspecified SQLite tie-break order.
        val service = DatabaseService(context)
        val ids = (1..4).map { i ->
            val id = recordPendingReceive(service, "same-second-$i")
            setCreatedAt(service, id, 5_000L) // identical timestamp for all rows
            id
        }

        val rows = service.getPendingOnchainReceives(limit = 3)
        // Newest-N by (created_at DESC, id DESC) keeps the 3 highest ids; oldest-first display
        // by (created_at ASC, id ASC) then lists them in ascending id order.
        assertEquals(ids.takeLast(3), rows.map { it.id })
        service.close()
    }

    private fun recordPendingReceive(service: DatabaseService, paymentId: String): Long =
        service.recordPayment(
            paymentId = paymentId, paymentType = "onchain", direction = "received",
            amountMsat = 100_000, status = "pending"
        )

    private fun setCreatedAt(service: DatabaseService, rowId: Long, createdAt: Long) {
        service.writableDatabase.execSQL(
            "UPDATE payments SET created_at = ? WHERE id = ?",
            arrayOf<Any>(createdAt, rowId)
        )
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) assertTrue(file.delete()) }
    }
}
