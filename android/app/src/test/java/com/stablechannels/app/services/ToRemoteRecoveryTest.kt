package com.stablechannels.app.services

import fr.acinq.bitcoin.ByteVector
import fr.acinq.bitcoin.Transaction
import java.util.Collections
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ToRemoteRecoveryTest {
    companion object {
        private val keys by lazy {
            ToRemoteKeys.fromMnemonic(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
            )
        }
    }

    private val funding = "aa".repeat(32)
    private val close = "7b0abcb56f5196eaa8ec0db3ddad1e1a7980ca8c428530b95c39eb9380c89243"
    private val ours = "0020976a6b80c817104437bd4024cfd4b44fe495a0b8061bb61998e690b695546811"
    private val lspOutput = """{"scriptpubkey":"0014${"11".repeat(20)}","value":5000}"""
    private val address = "bc1qmf6gnapjkfg4pg675v7y2l94vtv7s4evevr40l"

    private val routes = mutableMapOf<String, String>()
    // Paths the explorer says it doesn't have. Any other unknown path is an explorer error.
    private val missing = mutableSetOf<String>()
    private val posted = mutableListOf<String>()
    // Requests arrive on the server's threads.
    private val requested = Collections.synchronizedList(mutableListOf<String>())
    private var postStatus = 200
    private var addressRequests = 0
    private var feeRate: Long? = 2L
    private var onRequest: (String) -> Unit = {}
    private val swept = mutableListOf<Triple<String, Long, String>>()
    private val server =
        MockWebServer().apply {
            dispatcher =
                object : Dispatcher() {
                    override fun dispatch(request: RecordedRequest): MockResponse {
                        val path = request.path.orEmpty()
                        requested += path
                        onRequest(path)
                        if (request.method == "POST") {
                            posted += request.body.readUtf8()
                            return MockResponse().setResponseCode(postStatus).setBody("sweep")
                        }
                        routes[path]?.let {
                            return MockResponse().setBody(it)
                        }
                        return MockResponse().setResponseCode(if (path in missing) 404 else 503)
                    }
                }
            start()
        }
    private val http = OkHttpClient.Builder().callTimeout(2, TimeUnit.SECONDS).build()
    private var saved: String? = null
    private var clock = 1_000_000L

    @After
    fun tearDown() {
        server.shutdown()
    }

    private fun recovery() =
        ToRemoteRecovery(
            http,
            chainUrls = { listOf(server.url("/").toString()) },
            loadState = { saved },
            saveState = { saved = it },
            now = { clock },
            log = { _, _ -> },
            onSwept = { txid, sats, to -> swept += Triple(txid, sats, to) },
        )

    private fun ToRemoteRecovery.runNow() = runBlocking {
        run(
            { keys },
            {
                addressRequests++
                address
            },
            { feeRate },
        )
    }

    private fun closeConfirmed() {
        routes["/tx/$close/outspend/3"] = """{"spent":false}"""
        routes["/tx/$close/status"] = """{"confirmed":true}"""
    }

    private fun afterSettleInterval() {
        clock += ToRemoteRecovery.SETTLE_INTERVAL_MS
    }

    private fun closePaysUs(sats: Long = 94_467) {
        routes["/tx/$funding/outspends"] = """[{"spent":true,"txid":"$close","vin":0}]"""
        routes["/tx/$close"] =
            """{"vout":[$lspOutput,$lspOutput,$lspOutput,{"scriptpubkey":"$ours","value":$sats}]}"""
        routes["/tx/$close/outspends"] =
            """[{"spent":false},{"spent":false},{"spent":false},{"spent":false}]"""
    }

    @Test
    fun findsTheCloseFromTheFundingTxidAndSweepsItOnceConfirmed() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertEquals(listOf(ToRemoteRecovery.Output(close, 3, 94_467, 45)), recovery.state.found)
        assertEquals(94_467, recovery.pendingSats)

        // Not swept while the close is unconfirmed.
        routes["/tx/$close/outspend/3"] = """{"spent":false}"""
        routes["/tx/$close/status"] = """{"confirmed":false}"""
        recovery.runNow()
        assertTrue(posted.isEmpty())

        routes["/tx/$close/status"] = """{"confirmed":true,"block_height":900000}"""
        clock += ToRemoteRecovery.SETTLE_INTERVAL_MS
        recovery.runNow()
        val sweep = Transaction.read(posted.single())
        assertEquals(close, sweep.txIn.single().outPoint.txid.toString())
        assertEquals(3L, sweep.txIn.single().outPoint.index)
        assertEquals(1L, sweep.txIn.single().sequence)
        assertEquals(
            "0014da7489f432b25150a35ea33c457cb562d9e8572c",
            sweep.txOut.single().publicKeyScript.toHex(),
        )
        assertTrue(94_467 - sweep.txOut.single().amount.sat >= (sweep.weight() + 3) / 4 * 2L)

        // Counted until the sweep confirms, then done.
        routes["/tx/$close/outspend/3"] =
            """{"spent":true,"txid":"sweep","status":{"confirmed":false}}"""
        clock += ToRemoteRecovery.SETTLE_INTERVAL_MS
        recovery.runNow()
        assertEquals(94_467, recovery.pendingSats)
        routes["/tx/$close/outspend/3"] =
            """{"spent":true,"txid":"sweep","status":{"confirmed":true}}"""
        clock += ToRemoteRecovery.SETTLE_INTERVAL_MS
        recovery.runNow()
        assertEquals(0, recovery.pendingSats)
        assertFalse(recovery.state.active)
        assertEquals(1, posted.size)
        assertFalse(recovery.needsCheck(funding))
    }

    @Test
    fun aCloseThatDoesNotPayUsEndsTheCheck() {
        routes["/tx/$funding/outspends"] = """[{"spent":true,"txid":"$close","vin":0}]"""
        routes["/tx/$close"] = """{"vout":[$lspOutput]}"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertFalse(recovery.state.active)
        assertEquals(funding, recovery.state.checkedFundingTxid)
    }

    @Test
    fun anOutputAlreadySweptIsLeftOut() {
        closePaysUs()
        routes["/tx/$close/outspends"] =
            """[{"spent":false},{"spent":false},{"spent":false},{"spent":true,"txid":"x","status":{"confirmed":true}}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertTrue(recovery.state.found.isEmpty())
        assertFalse(recovery.state.active)
    }

    @Test
    fun unspentFundingIsRetriedLater() {
        routes["/tx/$funding/outspends"] = """[{"spent":false}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertTrue(recovery.state.active)
        assertEquals(clock + ToRemoteRecovery.backoffMs(1), recovery.state.nextAt)

        // Not due yet: no new request.
        val requests = server.requestCount
        recovery.runNow()
        assertEquals(requests, server.requestCount)

        closePaysUs()
        clock += ToRemoteRecovery.backoffMs(1)
        recovery.runNow()
        assertEquals(94_467, recovery.pendingSats)
    }

    @Test
    fun stateSurvivesARestart() {
        closePaysUs()
        recovery().apply {
            start(funding, 0)
            runNow()
        }
        assertEquals(94_467, recovery().pendingSats)
        assertNull(recovery().state.fundingTxid)
    }

    @Test
    fun backoffStartsAtAMinuteThenDoublesUpToADay() {
        assertEquals(60 * 1000L, ToRemoteRecovery.backoffMs(1))
        assertEquals(10 * 60 * 1000L, ToRemoteRecovery.backoffMs(2))
        assertEquals(20 * 60 * 1000L, ToRemoteRecovery.backoffMs(3))
        assertEquals(24 * 60 * 60 * 1000L, ToRemoteRecovery.backoffMs(20))
    }

    /** Case B without a restore: LDK lost the channel but the funding txid is still saved. */
    @Test
    fun lostChannelDataIsCheckedOnRefresh() {
        val recovery = recovery()
        assertFalse(recovery.onRefresh(funding, 0, ldkHasChannels = true, ldkHandlesClose = false))
        assertFalse(recovery.onRefresh(funding, 0, ldkHasChannels = false, ldkHandlesClose = true))
        assertFalse(recovery.state.active)

        assertTrue(recovery.onRefresh(funding, 0, ldkHasChannels = false, ldkHandlesClose = false))
        assertEquals(funding, recovery.state.fundingTxid)
        closePaysUs()
        recovery.runNow()
        assertEquals(94_467, recovery.pendingSats)

        // Never runs while LDK handles a close itself.
        assertFalse(recovery.onRefresh(null, null, ldkHasChannels = false, ldkHandlesClose = true))
        assertTrue(recovery.onRefresh(null, null, ldkHasChannels = false, ldkHandlesClose = false))
    }

    @Test
    fun aCheckedFundingTxidIsNotLookedUpAgain() {
        routes["/tx/$funding/outspends"] = """[{"spent":true,"txid":"$close","vin":0}]"""
        routes["/tx/$close"] = """{"vout":[$lspOutput]}"""
        val recovery = recovery()
        assertTrue(recovery.onRefresh(funding, 0, ldkHasChannels = false, ldkHandlesClose = false))
        recovery.runNow()
        val requests = requested.size
        assertFalse(recovery.onRefresh(funding, 0, ldkHasChannels = false, ldkHandlesClose = false))
        recovery.runNow()
        assertEquals(requests, requested.size)
    }

    @Test
    fun explorerDownKeepsTheSearchGoing() {
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertEquals(funding, recovery.state.fundingTxid)
        assertEquals(1, recovery.state.attempts)
        assertEquals(clock + ToRemoteRecovery.backoffMs(1), recovery.state.nextAt)
    }

    @Test
    fun searchGivesUpAfterTwoWeeks() {
        routes["/tx/$funding/outspends"] = """[{"spent":false}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertTrue(recovery.state.active)
        clock += ToRemoteRecovery.SEARCH_MS
        recovery.runNow()
        assertFalse(recovery.state.active)
        assertEquals(funding, recovery.state.checkedFundingTxid)
    }

    @Test
    fun knownVoutChecksOnlyTheFundingOutput() {
        routes["/tx/$funding/outspends"] = """[{"spent":false},{"spent":true,"txid":"other"}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertTrue(requested.none { it == "/tx/other" })
        assertEquals(funding, recovery.state.fundingTxid)
    }

    @Test
    fun failedBroadcastIsRetriedWithTheSameAddress() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        postStatus = 400
        recovery.runNow()
        assertEquals(1, posted.size)
        assertEquals(94_467, recovery.pendingSats)

        postStatus = 200
        afterSettleInterval()
        recovery.runNow()
        assertEquals(2, posted.size)
        assertEquals(posted[0], posted[1])
        assertEquals(1, addressRequests)
    }

    /** History gets the close from the sweep, as LDK never reports it (#364). */
    @Test
    fun onlyABroadcastSweepIsReported() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        postStatus = 400
        recovery.runNow()
        assertTrue(swept.isEmpty())

        postStatus = 200
        afterSettleInterval()
        recovery.runNow()
        val sweep = Transaction.read(posted.last())
        assertEquals(
            listOf(Triple(sweep.txid.toString(), sweep.txOut.single().amount.sat, address)),
            swept,
        )
    }

    @Test
    fun noFeeRateMeansNoBroadcastYet() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        feeRate = null
        recovery.runNow()
        assertTrue(posted.isEmpty())
        feeRate = 2L
        afterSettleInterval()
        recovery.runNow()
        assertEquals(1, posted.size)
    }

    @Test
    fun dustIsDroppedWithoutABroadcast() {
        routes["/tx/$funding/outspends"] = """[{"spent":true,"txid":"$close","vin":0}]"""
        routes["/tx/$close"] =
            """{"vout":[$lspOutput,$lspOutput,$lspOutput,{"scriptpubkey":"$ours","value":400}]}"""
        routes["/tx/$close/outspends"] =
            """[{"spent":false},{"spent":false},{"spent":false},{"spent":false}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertEquals(400, recovery.pendingSats)
        closeConfirmed()
        feeRate = 5L
        recovery.runNow()
        assertTrue(posted.isEmpty())
        assertFalse(recovery.state.active)
    }

    @Test
    fun aFeeOverATenthOfTheOutputWaits() {
        closePaysUs(3_000)
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        // About 550 sats at 5 sat/vB.
        feeRate = 5L
        recovery.runNow()
        assertTrue(posted.isEmpty())
        assertEquals(3_000, recovery.pendingSats)

        feeRate = 2L
        afterSettleInterval()
        recovery.runNow()
        assertEquals(1, posted.size)
    }

    /** Even 1 sat/vB costs over a tenth of 800 sats, so waiting for lower fees would never end. */
    @Test
    fun anOutputThatCannotWaitSweepsAt1SatVb() {
        closePaysUs(800)
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        feeRate = 5L
        recovery.runNow()
        val script = ByteVector("0014da7489f432b25150a35ea33c457cb562d9e8572c").toByteArray()
        val atFloor = ToRemoteSweep.build(keys.keys[45], close, 3, 800, script, 1)!!
        assertEquals(listOf(ByteVector(Transaction.write(atFloor)).toHex()), posted)
    }

    @Test
    fun theFeeRateIsCappedAtWhatAManualSendMayPay() {
        closePaysUs(100_000_000)
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        feeRate = 50_000L
        recovery.runNow()
        // Times the size, this would overflow.
        feeRate = Long.MAX_VALUE
        afterSettleInterval()
        recovery.runNow()
        assertEquals(2, posted.size)
        for (hex in posted) {
            val sweep = Transaction.read(hex)
            val vsize = (sweep.weight() + 3) / 4
            val fee = 100_000_000 - sweep.txOut.single().amount.sat
            assertTrue("fee $fee", fee in vsize * 10_000L..(vsize + 2) * 10_000L)
        }
    }

    @Test
    fun aStartDuringASearchIsKept() {
        val other = "cc".repeat(32)
        routes["/tx/$funding/outspends"] = """[{"spent":false}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        onRequest = {
            if (it == "/tx/$funding/outspends") recovery.start(other, 1)
        }
        recovery.runNow()
        assertEquals(other, recovery.state.fundingTxid)
        assertEquals(0, recovery.state.attempts)
        assertEquals(0L, recovery.state.nextAt)
    }

    @Test
    fun aMalformedReplyDoesNotCrash() {
        routes["/tx/$funding/outspends"] = """[{"spent":true}]"""
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        assertTrue(recovery.state.active)
    }

    @Test
    fun withoutAVoutTheChannelOutputIsFoundByItsWitness() {
        val multisig = "5221" + "02".padEnd(66, '1') + "21" + "03".padEnd(66, '2') + "52ae"
        // Output 0 is the LSP's change, still unspent; output 1 is the channel, closed by coop.
        routes["/tx/$funding/outspends"] =
            """[{"spent":false},{"spent":true,"txid":"$close","vin":0}]"""
        routes["/tx/$close"] =
            """{"vin":[{"txid":"$funding","vout":1,"witness":["","30","30","$multisig"]}],"vout":[$lspOutput]}"""
        val recovery = recovery()
        recovery.start(funding, null)
        recovery.runNow()
        assertFalse(recovery.state.active)
        assertEquals(funding, recovery.state.checkedFundingTxid)
    }

    @Test
    fun withoutAVoutAnUnrelatedSpendDoesNotEndTheCheck() {
        routes["/tx/$funding/outspends"] =
            """[{"spent":false},{"spent":true,"txid":"$close","vin":0}]"""
        routes["/tx/$close"] =
            """{"vin":[{"txid":"$funding","vout":1,"witness":["30","02aa"]}],"vout":[$lspOutput]}"""
        val recovery = recovery()
        recovery.start(funding, null)
        recovery.runNow()
        assertEquals(funding, recovery.state.fundingTxid)
    }

    @Test
    fun aZeroFeeEstimateSweepsAtTheFloor() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        feeRate = 0L
        recovery.runNow()
        val sweep = Transaction.read(posted.single())
        assertTrue(94_467 - sweep.txOut.single().amount.sat >= (sweep.weight() + 3) / 4)
    }

    /** After a restore the saved funding txid is checked until the LSP's close shows up. */
    @Test
    fun aRestoreWaitsForTheLspCloseThenSweeps() {
        routes["/tx/$funding/outspends"] = """[{"spent":false}]"""
        val recovery = recovery()
        recovery.startAfterRestore(funding, 0, repeat = true)
        recovery.runNow()
        assertTrue(recovery.state.active)
        assertEquals(0, recovery.pendingSats)

        // The LSP closes after the restored node reconnects.
        closePaysUs()
        clock += ToRemoteRecovery.backoffMs(1)
        recovery.runNow()
        assertEquals(94_467, recovery.pendingSats)

        closeConfirmed()
        afterSettleInterval()
        recovery.runNow()
        assertEquals(1, posted.size)

        routes["/tx/$close/outspend/3"] =
            """{"spent":true,"txid":"sweep","status":{"confirmed":true}}"""
        afterSettleInterval()
        recovery.runNow()
        assertFalse(recovery.state.active)
    }

    @Test
    fun aOneTimeRestoreCheckStopsWhenNothingIsFound() {
        routes["/tx/$funding/outspends"] = """[{"spent":false}]"""
        val recovery = recovery()
        recovery.startAfterRestore(funding, 0, repeat = false)
        recovery.runNow()
        assertFalse(recovery.state.active)
    }

    @Test
    fun aOneTimeRestoreCheckIsRetriedWhileTheExplorerIsDown() {
        val recovery = recovery()
        recovery.startAfterRestore(funding, 0, repeat = false)
        recovery.runNow()
        assertEquals(funding, recovery.state.fundingTxid)

        closePaysUs()
        clock += ToRemoteRecovery.backoffMs(1)
        recovery.runNow()
        assertEquals(94_467, recovery.pendingSats)
    }

    @Test
    fun aFundingTxNoExplorerKnowsIsNotSpent() {
        missing += "/tx/$funding/outspends"
        val recovery = recovery()
        recovery.startAfterRestore(funding, 0, repeat = false)
        recovery.runNow()
        assertFalse(recovery.state.active)
        assertEquals(funding, recovery.state.checkedFundingTxid)
    }

    @Test
    fun aRestoreDuringASweepStopsIt() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        // The restore lands while the sweep asks the node for an address. It looks for the close
        // of the saved funding txid again, with the restored wallet's keys.
        runBlocking {
            recovery.run(
                { keys },
                {
                    recovery.startAfterRestore(funding, 0, repeat = true)
                    address
                },
                { feeRate },
            )
        }
        assertNull(recovery.state.destination)
        assertTrue(posted.isEmpty())
        assertTrue(swept.isEmpty())
        // Not searched with the replaced wallet's keys.
        assertTrue(recovery.state.found.isEmpty())
        assertEquals(funding, recovery.state.fundingTxid)
    }

    @Test
    fun aNativeLibraryThatFailsToLoadDoesNotCrash() {
        val recovery = recovery()
        recovery.start(funding, 0)
        // As when R8 strips secp256k1's JNI loader.
        runBlocking {
            recovery.run({ throw ExceptionInInitializerError() }, { address }, { feeRate })
        }
        assertTrue(recovery.state.active)
    }

    @Test
    fun aRestoreWithoutAFundingTxidLooksForNothing() {
        val recovery = recovery()
        recovery.startAfterRestore(null, null, repeat = true)
        recovery.runNow()
        assertFalse(recovery.state.active)
        assertTrue(requested.isEmpty())
    }

    @Test
    fun aRestoreStartsOver() {
        closePaysUs()
        val recovery = recovery()
        recovery.start(funding, 0)
        recovery.runNow()
        closeConfirmed()
        recovery.runNow()
        assertEquals(address, recovery.state.destination)

        val other = "dd".repeat(32)
        recovery.startAfterRestore(other, 1, repeat = true)
        assertTrue(recovery.state.found.isEmpty())
        assertNull(recovery.state.destination)
        assertNull(recovery.state.checkedFundingTxid)
        assertEquals(other, recovery.state.fundingTxid)
    }
}
