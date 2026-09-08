package com.stablechannels.app.services

import android.content.Context
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.runBlocking
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
import java.net.ServerSocket

/**
 * Regression test for #264: a channel-close "Received onchain" row got permanently stuck at
 * 0/6 confirmations because CloseTxidResolver hardcoded vout=0 when polling Esplora's
 * /tx/{fundingTxid}/outspend/{vout}, so a funding output at any other index could never be
 * found. Uses a plain ServerSocket (no new test dependency, and JDK's HttpServer isn't on the
 * Android unit-test classpath) so the assertion is against the actual URL requested, not a
 * mocked stand-in for it.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CloseTxidResolverTest {
    private lateinit var context: Context
    private lateinit var dbFile: File
    private lateinit var server: ServerSocket

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        deleteDatabaseFiles()
    }

    @After
    fun tearDown() {
        if (::server.isInitialized) server.close()
        deleteDatabaseFiles()
    }

    /** Accepts exactly one HTTP connection, records the request line's path, and replies with a
     *  fixed spent-outpoint JSON body. Runs on a background thread since ServerSocket.accept()
     *  blocks. */
    private fun startFakeEsplora(spentTxid: String): Pair<String, () -> String?> {
        server = ServerSocket(0)
        var requestPath: String? = null
        Thread {
            try {
                server.accept().use { socket ->
                    val reader = socket.getInputStream().bufferedReader()
                    val requestLine = reader.readLine() ?: ""
                    requestPath = requestLine.split(" ").getOrNull(1)
                    // Drain remaining headers so the client isn't left hanging on the request.
                    while (true) {
                        val line = reader.readLine()
                        if (line.isNullOrEmpty()) break
                    }
                    val body = """{"spent": true, "txid": "$spentTxid"}"""
                    val response = "HTTP/1.1 200 OK\r\n" +
                        "Content-Type: application/json\r\n" +
                        "Content-Length: ${body.toByteArray().size}\r\n" +
                        "Connection: close\r\n\r\n" + body
                    socket.getOutputStream().write(response.toByteArray())
                    socket.getOutputStream().flush()
                }
            } catch (_: Exception) {
                // Socket closed by tearDown or the resolver only made one poll attempt; either
                // way there's nothing left to serve.
            }
        }.start()
        return "http://127.0.0.1:${server.localPort}" to { requestPath }
    }

    @Test
    fun resolvesUsingRealVoutNotHardcodedZero() = runBlocking {
        val fundingTxid = "a".repeat(64)
        val closeTxid = "b".repeat(64)
        val (baseUrl, requestedPath) = startFakeEsplora(closeTxid)

        val service = DatabaseService(context)
        service.recordPayment(
            paymentId = "close-row", paymentType = "channel_close", direction = "received",
            amountMsat = 50_000_000, status = "pending", txid = null
        )

        var resolvedTxid: String? = null
        val resolver = CloseTxidResolver(
            chainURLs = listOf(baseUrl),
            onResolved = { _, txid -> resolvedTxid = txid }
        )
        resolver.resolve(
            paymentId = "close-row",
            fundingTxid = fundingTxid,
            vout = 3,
            databaseService = service
        )

        assertEquals("/tx/$fundingTxid/outspend/3", requestedPath())
        assertEquals(closeTxid, resolvedTxid)
        assertTrue(service.getRecentPayments(10).single { it.paymentId == "close-row" }.txid == closeTxid)
        service.close()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) file.delete() }
    }
}
