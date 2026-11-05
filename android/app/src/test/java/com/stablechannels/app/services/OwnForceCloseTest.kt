package com.stablechannels.app.services

import android.database.sqlite.SQLiteDatabase
import java.io.File
import java.nio.file.Files
import java.util.Collections
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.sc_force_close.ForceCloseTransactions

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OwnForceCloseTest {
    private val funding = "aa".repeat(32)
    private val outspendPath = "/tx/$funding/outspend/1"
    private val close = ForceCloseTransactions("c1", "c1hex", "b1", "b1hex")

    private var outspend: MockResponse? = MockResponse().setBody("""{"spent":false}""")
    /** Whether the explorer has the funding tx. */
    private var hasFunding = true
    private var postStatus = 200
    // Requests arrive on the server's threads.
    private val posted = Collections.synchronizedList(mutableListOf<String>())
    private val server =
        MockWebServer().apply {
            dispatcher =
                object : Dispatcher() {
                    override fun dispatch(request: RecordedRequest): MockResponse {
                        if (request.method == "POST") {
                            posted += request.body.readUtf8()
                            return MockResponse().setResponseCode(postStatus).setBody("txid")
                        }
                        if (request.path == outspendPath) {
                            outspend?.let {
                                return it
                            }
                        }
                        if (request.path == "/tx/$funding/hex") {
                            return if (hasFunding) MockResponse().setBody("0200")
                            else MockResponse().setResponseCode(404)
                        }
                        return MockResponse().setResponseCode(503)
                    }
                }
            start()
        }
    /** A second explorer, used when [useOther] is set. */
    private var otherOutspend: MockResponse? = null
    private var useOther = false
    private val otherPosted = Collections.synchronizedList(mutableListOf<String>())
    private val other =
        MockWebServer().apply {
            dispatcher =
                object : Dispatcher() {
                    override fun dispatch(request: RecordedRequest): MockResponse {
                        if (request.method == "POST") {
                            otherPosted += request.body.readUtf8()
                            return MockResponse().setBody("txid")
                        }
                        return otherOutspend ?: MockResponse().setResponseCode(503)
                    }
                }
            start()
        }
    private val http = OkHttpClient.Builder().callTimeout(2, TimeUnit.SECONDS).build()
    private var saved: String? = null
    private var clock = 1_000_000L
    private val logged = mutableListOf<Pair<String, Map<String, Any?>>>()
    private var builds = 0
    private var buildError: Exception? = null
    private val confirmed = mutableListOf<Pair<String, String>>()

    @After
    fun tearDown() {
        server.shutdown()
        other.shutdown()
    }

    private fun ownForceClose() =
        OwnForceClose(
            http,
            chainUrls = {
                listOfNotNull(
                    server.url("/").toString(),
                    other.url("/").toString().takeIf { useOther },
                )
            },
            loadState = { saved },
            saveState = { saved = it },
            now = { clock },
            log = { event, data -> logged += event to data },
        )

    private fun OwnForceClose.runNow() = runBlocking {
        step(
            build = {
                builds++
                buildError?.let { throw it }
                close
            },
            onConfirmed = { channelId, txid -> confirmed += channelId to txid },
        )
    }

    @Test
    fun broadcastsTheCloseThenTheFeeBumpWhenNoCloseShowsUp() {
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        val message = forceClose.runNow()

        assertEquals(1, builds)
        assertEquals(listOf("c1hex", "b1hex"), posted)
        assertEquals("Close sent.", message)
        val state = forceClose.state.value!!
        assertEquals("c1hex", state.closeHex)
        assertEquals("b1hex", state.feeBumpHex)
        assertEquals(clock, state.broadcastAt)
        assertEquals(clock + OwnForceClose.CHECK_MS, state.nextAt)
        assertEquals("OWN_FORCE_CLOSE_BROADCAST", logged.single().first)
    }

    @Test
    fun broadcastsAgainWithoutBuildingAgainWhileNoCloseShowsUp() {
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)
        forceClose.runNow()
        clock += OwnForceClose.CHECK_MS
        forceClose.runNow()
        assertEquals(listOf("c1hex", "b1hex"), posted)
        clock += OwnForceClose.RETRY_MS

        forceClose.runNow()

        assertEquals(1, builds)
        assertEquals(listOf("c1hex", "b1hex", "c1hex", "b1hex"), posted)
    }

    @Test
    fun waitsWhileAClosePaysInTheMempool() {
        outspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"lsp","vin":0,"status":{"confirmed":false}}""")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        assertNull(forceClose.runNow())

        assertEquals(1, builds)
        assertEquals(emptyList<String>(), posted)
        assertEquals(clock + OwnForceClose.CHECK_MS, forceClose.state.value!!.nextAt)
    }

    @Test
    fun stopsOnceACloseConfirms() {
        outspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"lsp","vin":0,"status":{"confirmed":true}}""")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        forceClose.runNow()

        assertNull(forceClose.state.value)
        assertNull(saved)
        assertEquals(
            "OWN_FORCE_CLOSE_DONE" to "lsp",
            logged.single().let {
                it.first to it.second["close_txid"]
            },
        )
        assertEquals(listOf("ch" to "lsp"), confirmed)
    }

    @Test
    fun waitsWhileNoExplorerAnswers() {
        outspend = null
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        assertNull(forceClose.runNow())

        // Building needs no explorer.
        assertEquals(1, builds)
        assertEquals(emptyList<String>(), posted)
        assertEquals(clock + OwnForceClose.CHECK_MS, forceClose.state.value!!.nextAt)
    }

    @Test
    fun waitsWhileNoExplorerHasTheFunding() {
        // Esplora reports an output of a tx it does not have as unspent.
        hasFunding = false
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        assertNull(forceClose.runNow())

        assertEquals(emptyList<String>(), posted)
        assertEquals(clock + OwnForceClose.CHECK_MS, forceClose.state.value!!.nextAt)
    }

    @Test
    fun triesAgainLaterWhenTheCloseCannotBeBuilt() {
        buildError = IllegalStateException("node data owned by stability-service")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        val message = forceClose.runNow()

        assertEquals("Close not created yet. Retrying in 10 minutes.", message)
        val state = forceClose.state.value!!
        assertNull(state.closeHex)
        assertEquals(clock + OwnForceClose.RETRY_MS, state.nextAt)
        assertEquals(
            "java.lang.IllegalStateException: node data owned by stability-service",
            logged.single().second["error"],
        )
    }

    @Test
    fun saysSoWhenNoExplorerAcceptsTheClose() {
        postStatus = 400
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        assertEquals("Close not accepted yet. Retrying in 10 minutes.", forceClose.runNow())
    }

    @Test
    fun offersASpeedUpOnlyWhileOurCloseIsSlow() {
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)
        forceClose.runNow()
        val sentAt = clock
        // No explorer has our close yet: nothing to speed up.
        assertEquals(false, forceClose.state.value!!.isSlow(sentAt + OwnForceClose.SLOW_MS))

        outspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"c1","vin":0,"status":{"confirmed":false}}""")
        clock += OwnForceClose.CHECK_MS
        forceClose.runNow()
        // Timed from when an explorer first showed our close.
        val seenAt = clock
        clock += OwnForceClose.CHECK_MS
        forceClose.runNow()
        assertEquals(false, forceClose.state.value!!.isSlow(seenAt + OwnForceClose.SLOW_MS - 1))
        assertEquals(true, forceClose.state.value!!.isSlow(seenAt + OwnForceClose.SLOW_MS))

        // The LSP's close replaced ours in the mempool: speeding ours up can't help.
        outspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"lsp","vin":0,"status":{"confirmed":false}}""")
        clock += OwnForceClose.CHECK_MS
        forceClose.runNow()
        assertEquals(false, forceClose.state.value!!.isSlow(seenAt + OwnForceClose.SLOW_MS))
    }

    @Test
    fun offersASpeedUpWhileOnlySomeExplorersHaveOurClose() {
        useOther = true
        otherOutspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"c1","vin":0,"status":{"confirmed":false}}""")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        // The explorer without our close gets it again; the one that has it doesn't.
        assertNull(forceClose.runNow())
        clock += OwnForceClose.RETRY_MS
        assertNull(forceClose.runNow())

        assertEquals(listOf("c1hex", "b1hex", "c1hex", "b1hex"), posted)
        assertEquals(emptyList<String>(), otherPosted)
        val seenAt = clock - OwnForceClose.RETRY_MS
        assertEquals(true, forceClose.state.value!!.isSlow(seenAt + OwnForceClose.SLOW_MS))
    }

    @Test
    fun stopsAfterTheCloseCannotBeBuiltThreeTimes() {
        buildError = IllegalStateException("channel not found")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)
        repeat(OwnForceClose.MAX_BUILD_FAILURES - 1) {
            assertEquals("Close not created yet. Retrying in 10 minutes.", forceClose.runNow())
            clock += OwnForceClose.RETRY_MS
        }

        val message = forceClose.runNow()

        assertEquals(
            "Close could not be created. Funds arrive once the provider closes the channel.",
            message,
        )
        assertEquals(OwnForceClose.MAX_BUILD_FAILURES, builds)
        assertNull(forceClose.state.value)
        assertNull(saved)
    }

    @Test
    fun sendsOurCloseToTheExplorersThatSeeTheFundingUnspent() {
        useOther = true
        otherOutspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"x","vin":0,"status":{"confirmed":false}}""")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        // Another explorer already has a close of the channel, so there is nothing new to tell.
        assertNull(forceClose.runNow())

        assertEquals(1, builds)
        assertEquals(listOf("c1hex", "b1hex"), posted)
        assertEquals(emptyList<String>(), otherPosted)
        assertEquals("x", forceClose.state.value!!.spendTxid)
    }

    @Test
    fun confirmsOnlyWhenEveryExplorerThatAnswersAgrees() {
        useOther = true
        outspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"lsp","vin":0,"status":{"confirmed":true}}""")
        otherOutspend =
            MockResponse()
                .setBody("""{"spent":true,"txid":"lsp","vin":0,"status":{"confirmed":false}}""")
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        forceClose.runNow()

        assertEquals("lsp", forceClose.state.value!!.spendTxid)
        assertEquals(emptyList<Pair<String, String>>(), confirmed)

        otherOutspend = outspend
        clock += OwnForceClose.CHECK_MS
        forceClose.runNow()

        assertNull(forceClose.state.value)
        assertEquals(listOf("ch" to "lsp"), confirmed)
    }

    @Test
    fun keepsAForceCloseStartedDuringAStep() {
        val forceClose = ownForceClose()
        forceClose.start("ch", funding, 1)

        runBlocking {
            forceClose.step(
                build = {
                    forceClose.start("ch2", "bb".repeat(32), 0)
                    close
                }
            )
        }

        val state = forceClose.state.value!!
        assertEquals("ch2", state.channelId)
        assertNull(state.closeHex)
    }

    @Test
    fun keepsTheBuiltCloseAcrossRestarts() {
        ownForceClose().apply {
            start("ch", funding, 1)
            runNow()
        }

        val state = ownForceClose().state.value!!

        assertEquals("c1hex", state.closeHex)
        assertEquals("b1", state.feeBumpTxid)
    }

    @Test
    fun readsOnlyMonitorsAndTheirUpdates() {
        val dataDir: File = Files.createTempDirectory("own_force_close").toFile()
        SQLiteDatabase.openOrCreateDatabase(File(dataDir, "ldk_node_data.sqlite"), null).use { db ->
            db.execSQL(
                "CREATE TABLE ldk_node_data (primary_namespace TEXT NOT NULL, " +
                    "secondary_namespace TEXT NOT NULL, key TEXT NOT NULL, value BLOB)"
            )
            val insert = "INSERT INTO ldk_node_data VALUES (?, ?, ?, ?)"
            db.execSQL(insert, arrayOf("monitors", "", "aa_1", byteArrayOf(1, 2)))
            db.execSQL(insert, arrayOf("monitor_updates", "aa_1", "3", byteArrayOf(3)))
            db.execSQL(insert, arrayOf("", "", "manager", byteArrayOf(4)))
            db.execSQL(insert, arrayOf("archived_monitors", "", "bb_0", byteArrayOf(5)))
        }

        val values = OwnForceClose.readMonitors(dataDir).sortedBy { it.primaryNamespace }

        assertEquals(listOf("monitor_updates", "monitors"), values.map { it.primaryNamespace })
        assertEquals(listOf("aa_1", ""), values.map { it.secondaryNamespace })
        assertEquals(listOf("3", "aa_1"), values.map { it.key })
        assertArrayEquals(byteArrayOf(3), values[0].value)
        assertArrayEquals(byteArrayOf(1, 2), values[1].value)
    }

    @Test
    fun broadcastsToEveryExplorer() {
        val rejecting =
            MockWebServer().apply {
                enqueue(MockResponse().setResponseCode(400).setBody("min relay fee not met"))
            }
        val urls = listOf(server.url("/api").toString(), rejecting.url("/api").toString())

        val results = OwnForceClose.broadcast(http, urls, "0200")

        assertEquals("txid", results[urls[0]])
        assertEquals("error: 400 min relay fee not met", results[urls[1]])
        assertEquals(listOf("0200"), posted)
        assertEquals("/api/tx", rejecting.takeRequest().path)
        rejecting.shutdown()
    }
}
