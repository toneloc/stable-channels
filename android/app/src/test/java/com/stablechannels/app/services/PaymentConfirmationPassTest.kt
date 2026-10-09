package com.stablechannels.app.services

import android.content.Context
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.util.Constants
import java.io.File
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import okhttp3.mockwebserver.SocketPolicy
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * Exercises the production confirmation pass that AppState runs, against a real DatabaseService and
 * a mock block explorer.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PaymentConfirmationPassTest {
    private lateinit var context: Context
    private lateinit var dbFile: File
    private lateinit var db: DatabaseService
    private lateinit var server: MockWebServer
    private val routes = mutableMapOf<String, () -> MockResponse>()
    private val rowsUpdatedCalls = mutableListOf<Boolean>()

    // Generous client timeouts: a deadline must cancel stalled requests, not wait these out.
    private val httpClient =
        OkHttpClient.Builder()
            .connectTimeout(30, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .callTimeout(60, TimeUnit.SECONDS)
            .build()

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        deleteDatabaseFiles()
        db = DatabaseService(context)
        server = MockWebServer()
        server.dispatcher =
            object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse =
                    routes[request.path]?.invoke() ?: MockResponse().setResponseCode(404)
            }
        server.start()
    }

    @After
    fun tearDown() {
        httpClient.dispatcher.cancelAll()
        server.shutdown()
        db.close()
        deleteDatabaseFiles()
    }

    private fun pass(onRowsUpdated: (Boolean) -> Unit = { rowsUpdatedCalls += it }) =
        PaymentConfirmationPass(
            httpClient = httpClient,
            chainUrls = { listOf(server.url("/").toString()) },
            database = { db },
            onRowsUpdated = onRowsUpdated,
        )

    private fun ok(body: String) = { MockResponse().setResponseCode(200).setBody(body) }

    private val serverError = { MockResponse().setResponseCode(500) }

    private fun recordSent(paymentId: String, txid: String): Long =
        db.recordPayment(
            paymentId = paymentId,
            paymentType = "onchain",
            direction = "sent",
            amountMsat = 100_000,
            status = "pending",
            txid = txid,
        )

    private fun payment(rowId: Long): PaymentRecord =
        db.getRecentPayments().first { it.id == rowId }

    @Test
    fun `chain tip failure reports ChainTipUnavailable`() = runBlocking {
        recordSent("p1", "tx1")
        routes["/blocks/tip/height"] = serverError

        assertEquals(ConfirmationPollResult.ChainTipUnavailable, pass().run())
        assertEquals(1, server.requestCount)
    }

    @Test
    fun `failed transaction lookups are counted`() = runBlocking {
        recordSent("p1", "tx1")
        db.recordPayment(
            paymentId = "p2",
            paymentType = "onchain",
            direction = "received",
            amountMsat = 100_000,
            status = "pending",
            txid = "tx2",
            address = "bc1qexampleaddress",
        )
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1/status"] = serverError
        routes["/tx/tx2"] = serverError

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 2), pass().run())
        assertTrue(rowsUpdatedCalls.isEmpty())
    }

    @Test
    fun `tip 105 and block 100 gives six confirmations and completes the payment`() = runBlocking {
        val rowId = recordSent("p1", "tx1")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1/status"] = ok("""{"confirmed":true,"block_height":100}""")

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 0), pass().run())

        val row = payment(rowId)
        assertEquals(6, row.confirmations)
        assertEquals("completed", row.status)
        assertEquals(listOf(true), rowsUpdatedCalls)
    }

    private fun age(rowId: Long, days: Long) =
        db.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - ? WHERE id = ?",
            arrayOf(days * 86400L, rowId),
        )

    @Test
    fun `old row is failed only when every explorer says not found`() = runBlocking {
        val notFoundRow = recordSent("p1", "tx1").also { age(it, 20) }
        val outageRow = recordSent("p2", "tx2").also { age(it, 20) }
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx2/status"] = serverError

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 1), pass().run())

        assertEquals("failed", payment(notFoundRow).status)
        assertEquals("pending", payment(outageRow).status)
    }

    @Test
    fun `recent row the explorer does not know stays pending`() = runBlocking {
        val rowId = recordSent("p1", "tx1")
        routes["/blocks/tip/height"] = ok("105")

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 1), pass().run())

        assertEquals("pending", payment(rowId).status)
    }

    @Test
    fun `old receive row the explorer does not know is failed`() = runBlocking {
        val rowId = recordReceived("p1", "tx1", "bc1qexampleaddress").also { age(it, 20) }
        routes["/blocks/tip/height"] = ok("105")

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 0), pass().run())

        assertEquals("failed", payment(rowId).status)
        assertEquals(listOf(true), rowsUpdatedCalls)
    }

    private fun recordReceived(paymentId: String, txid: String, address: String): Long =
        db.recordPayment(
            paymentId = paymentId,
            paymentType = "onchain",
            direction = "received",
            amountMsat = 100_000,
            status = "pending",
            txid = txid,
            address = address,
        )

    private fun txBody(address: String, status: String? = null) =
        """{"vout":[{"scriptpubkey_address":"$address"}]""" +
            (status?.let { ""","status":$it""" } ?: "") +
            "}"

    private fun requestedPaths(): List<String> =
        (1..server.requestCount).map { server.takeRequest().path.orEmpty() }

    @Test
    fun `receive row uses the status from the single tx request`() = runBlocking {
        val rowId = recordReceived("r1", "tx1", "bc1qrecv")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1"] = ok(txBody("bc1qrecv", """{"confirmed":true,"block_height":100}"""))

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 0), pass().run())

        assertEquals(6, payment(rowId).confirmations)
        assertEquals("completed", payment(rowId).status)
        assertEquals(listOf("/blocks/tip/height", "/tx/tx1"), requestedPaths())
    }

    @Test
    fun `receive row falls back to the status request when the tx has no status`() = runBlocking {
        val rowId = recordReceived("r1", "tx1", "bc1qrecv")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1"] = ok(txBody("bc1qrecv"))
        routes["/tx/tx1/status"] = ok("""{"confirmed":true,"block_height":104}""")

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 0), pass().run())

        assertEquals(2, payment(rowId).confirmations)
        assertEquals(listOf("/blocks/tip/height", "/tx/tx1", "/tx/tx1/status"), requestedPaths())
    }

    @Test
    fun `receive tx paying a different address is cleared and not confirmed`() = runBlocking {
        val rowId = recordReceived("r1", "tx1", "bc1qrecv")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1"] = ok(txBody("bc1qother", """{"confirmed":true,"block_height":100}"""))
        val mismatches = mutableListOf<String>()
        val pass =
            PaymentConfirmationPass(
                httpClient = httpClient,
                chainUrls = { listOf(server.url("/").toString()) },
                database = { db },
                onReceiveTxidMismatch = { mismatches += it },
            )

        assertEquals(ConfirmationPollResult.Completed(failedLookups = 0), pass.run())

        assertEquals(listOf("tx1"), mismatches)
        assertEquals("pending", payment(rowId).status)
        assertEquals(listOf("/blocks/tip/height", "/tx/tx1"), requestedPaths())
    }

    @Test
    fun `manual refresh returns without waiting for the wallet sync`() = runBlocking {
        val rowId = recordSent("p1", "tx1")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1/status"] = ok("""{"confirmed":true,"block_height":100}""")
        // Mirrors AppState: an inline sync blocks the pass (here: forever) until released.
        val syncRelease = java.util.concurrent.CountDownLatch(1)
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) { manual ->
                pass { syncInline ->
                        rowsUpdatedCalls += syncInline
                        if (syncInline) syncRelease.await()
                    }
                    .run(manual)
            }

        try {
            assertEquals(
                ConfirmationPollResult.Completed(failedLookups = 0),
                coordinator.refresh(deadlineMs = 5_000),
            )
            assertEquals(listOf(false), rowsUpdatedCalls)
            assertEquals("completed", payment(rowId).status)
        } finally {
            syncRelease.countDown()
        }
    }

    @Test
    fun `automatic poll still syncs inline after updating rows`() = runBlocking {
        recordSent("p1", "tx1")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1/status"] = ok("""{"confirmed":true,"block_height":100}""")
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) { manual -> pass().run(manual) }

        assertEquals(
            ConfirmationPollResult.Completed(failedLookups = 0),
            coordinator.pollIfIdle(force = true),
        )
        assertEquals(listOf(true), rowsUpdatedCalls)
    }

    @Test
    fun `manual refresh deadline cancels a stalled lookup and skips remaining rows`() =
        runBlocking {
            val stalledRow = recordSent("p1", "tx1")
            recordSent("p2", "tx2")
            routes["/blocks/tip/height"] = ok("105")
            routes["/tx/tx1/status"] = { MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE) }
            routes["/tx/tx2/status"] = { MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE) }
            val coordinator =
                ConfirmationRefreshCoordinator(nowMs = { 0L }) { manual -> pass().run(manual) }

            val startedAt = System.nanoTime()
            val result = coordinator.refresh(deadlineMs = 1_000)
            val elapsedMs = TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - startedAt)

            assertEquals(ConfirmationPollResult.TimedOut, result)
            assertTrue("refresh took ${elapsedMs}ms", elapsedMs < 5_000)
            assertNoRunningCalls()
            // Tip + the first (stalled) lookup only; the second row was never attempted.
            assertEquals(2, server.requestCount)
            assertEquals(0, payment(stalledRow).confirmations)
            // The lock was released, so the next refresh runs.
            routes["/tx/tx1/status"] = ok("""{"confirmed":true,"block_height":100}""")
            routes["/tx/tx2/status"] = ok("""{"confirmed":false}""")
            assertEquals(
                ConfirmationPollResult.Completed(0),
                coordinator.refresh(deadlineMs = 5_000),
            )
            assertEquals(6, payment(stalledRow).confirmations)
        }

    @Test
    fun `deadline cancels a response whose body stalls`() = runBlocking {
        recordSent("p1", "tx1")
        routes["/blocks/tip/height"] = ok("105")
        routes["/tx/tx1/status"] = {
            MockResponse()
                .setResponseCode(200)
                .setBody("""{"confirmed":true,"block_height":100}""")
                .throttleBody(1, 2, TimeUnit.SECONDS)
        }
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) { manual -> pass().run(manual) }

        val startedAt = System.nanoTime()
        assertEquals(ConfirmationPollResult.TimedOut, coordinator.refresh(deadlineMs = 1_000))
        val elapsedMs = TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - startedAt)
        assertTrue("refresh took ${elapsedMs}ms", elapsedMs < 5_000)
        assertNoRunningCalls()
    }

    private fun assertNoRunningCalls() {
        val until = System.currentTimeMillis() + 2_000
        while (
            httpClient.dispatcher.runningCallsCount() > 0 && System.currentTimeMillis() < until
        ) {
            Thread.sleep(20)
        }
        assertEquals(0, httpClient.dispatcher.runningCallsCount())
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
        assertFalse(dbFile.exists())
    }
}
