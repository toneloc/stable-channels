package com.stablechannels.app.services.fees

import com.stablechannels.app.domain.send.NetworkFeeSpeedTier
import com.stablechannels.app.domain.send.RecommendedFees
import com.stablechannels.app.util.Constants
import java.io.IOException
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class FeeRateServiceTest {

    private var now = 0L
    private var fetchCount = 0

    private fun fees(rate: Double) =
        RecommendedFees(
            fastestFee = rate * 1.3,
            halfHourFee = rate,
            hourFee = rate * 0.8,
            minimumFee = rate * 0.5,
        )

    private fun counting(rate: Double, delayMillis: Long = 0) = FeeRateSource {
        fetchCount++
        if (delayMillis > 0) delay(delayMillis)
        fees(rate)
    }

    private val failing = FeeRateSource {
        fetchCount++
        throw IOException("timeout")
    }

    private fun TestScope.service(
        vararg sources: FeeRateSource,
        ttl: Long = 60_000,
        fallback: Double = 2.0,
    ) = FeeRateService(sources.toList(), ttl, fallback, backgroundScope) { now }

    // FeeRateService

    @Test
    fun `cache hit within ttl does not refetch`() = runTest {
        val service = service(counting(7.0))
        assertEquals(7.0, service.recommendedFees().halfHourFee, 0.0)
        now = 59_999
        assertEquals(7.0, service.recommendedFees().halfHourFee, 0.0)
        assertEquals(7.0, service.recommendedFees().halfHourFee, 0.0)
        assertEquals(1, fetchCount)
    }

    @Test
    fun `refetches after ttl`() = runTest {
        val service = service(counting(3.0))
        service.recommendedFees()
        now = 60_000
        service.recommendedFees()
        assertEquals(2, fetchCount)
    }

    @Test
    fun `concurrent callers share one fetch`() = runTest {
        val service = service(counting(11.0, delayMillis = 150))
        val results = List(3) { async { service.recommendedFees().halfHourFee } }.awaitAll()
        assertEquals(listOf(11.0, 11.0, 11.0), results)
        assertEquals(1, fetchCount)
    }

    @Test
    fun `faster good source beats slower good source`() = runTest {
        val service = service(counting(999.0, delayMillis = 300), counting(4.0, delayMillis = 20))
        assertEquals(4.0, service.recommendedFees().halfHourFee, 0.0)
    }

    @Test
    fun `first successful source wins when another fails`() = runTest {
        val service = service(failing, counting(19.0, delayMillis = 10))
        assertEquals(19.0, service.recommendedFees().halfHourFee, 0.0)
    }

    @Test
    fun `all sources fail returns fallback that is not cached`() = runTest {
        val service = service(failing, FeeRateSource { throw IllegalStateException("parse") })
        val first = service.recommendedFees()
        assertEquals(fees(2.0), first)
        assertEquals(2.6, first.fastestFee, 1e-9)
        assertEquals(2.0, first.halfHourFee, 0.0)
        assertEquals(1.6, first.hourFee, 1e-9)
        assertEquals(1.0, first.minimumFee, 0.0)
        assertEquals(1, fetchCount)

        assertEquals(fees(2.0), service.recommendedFees())
        assertEquals("fallback must not be cached", 2, fetchCount)
    }

    @Test
    fun `fallback uses configured rate`() = runTest {
        val service = service(failing, fallback = 9.0)
        assertEquals(9.0, service.recommendedFees().halfHourFee, 0.0)
        assertEquals(9.0, service.recommendedFees().halfHourFee, 0.0)
        assertEquals(2, fetchCount)
    }

    @Test
    fun `sub minimum fallback is clamped to min allowed rate`() = runTest {
        val fees = service(failing, fallback = 0.5).recommendedFees()
        assertTrue(fees.rate(NetworkFeeSpeedTier.ECONOMY) >= Constants.MIN_FEE_RATE_SAT_VB)
        assertTrue(fees.minimumFee >= Constants.MIN_FEE_RATE_SAT_VB)
    }

    @Test
    fun `all sources fail after a success keeps last good rates`() = runTest {
        var shouldFail = false
        val flaky = FeeRateSource {
            fetchCount++
            if (shouldFail) throw IOException("timeout")
            RecommendedFees(60.0, 45.0, 30.0, minimumFee = 15.0)
        }
        val service = service(flaky)
        assertEquals(45.0, service.recommendedFees().halfHourFee, 0.0)

        shouldFail = true
        now = 120_000
        assertEquals(45.0, service.recommendedFees().halfHourFee, 0.0)
        assertEquals(2, fetchCount)
    }

    @Test
    fun `update during in-flight fetch wins over fetch result`() = runTest {
        val service = service(counting(10.0, delayMillis = 120))
        val initiator = async { service.recommendedFees() }
        runCurrent()
        val waiter = async { service.recommendedFees() }
        runCurrent()

        val pushed = RecommendedFees(65.0, 50.0, 40.0, minimumFee = 25.0)
        service.update(pushed)

        assertEquals(pushed, initiator.await())
        assertEquals(pushed, waiter.await())
        assertEquals(pushed, service.recommendedFees())
        assertEquals(1, fetchCount)
    }

    @Test
    fun `update is served from cache without fetching`() = runTest {
        val service = service(counting(10.0))
        val pushed = RecommendedFees(65.0, 50.0, 40.0, minimumFee = 25.0)
        service.update(pushed)
        assertEquals(pushed, service.recommendedFees())
        assertEquals(0, fetchCount)
    }

    @Test
    fun `invalidate forces refetch`() = runTest {
        val service = service(counting(5.0))
        service.recommendedFees()
        assertEquals(1, fetchCount)
        service.invalidate()
        service.recommendedFees()
        assertEquals(2, fetchCount)
    }

    @Test
    fun `huge source rate is clamped to max allowed rate`() = runTest {
        val fees = service(counting(5_000_000.0)).recommendedFees()
        assertEquals(Constants.MAX_FEE_RATE_SAT_VB, fees.halfHourFee, 0.0)
    }

    // RecommendedFees

    @Test
    fun `recommended fees sanitizes hostile values and clamps ceiling`() {
        val hostile = RecommendedFees(-10.0, Double.NaN, Double.POSITIVE_INFINITY, 0.0, -1.0)
        assertEquals(1.0, hostile.minimumFee, 0.0)
        assertEquals(1.0, hostile.fastestFee, 0.0)
        assertEquals(1.0, hostile.halfHourFee, 0.0)
        assertEquals(1.0, hostile.hourFee, 0.0)
        assertEquals(1.0, hostile.economyFee!!, 0.0)

        val absurd = RecommendedFees(5_000_000.0, 100.0, 50.0, minimumFee = 10.0)
        assertEquals(Constants.MAX_FEE_RATE_SAT_VB, absurd.fastestFee, 0.0)
        assertEquals(Constants.MAX_FEE_RATE_SAT_VB, absurd.rate(NetworkFeeSpeedTier.PRIORITY), 0.0)
    }

    @Test
    fun `minimum fee anomaly is capped against half hour fee`() {
        val anomaly = RecommendedFees(20.0, 10.0, 5.0, 3.0, 500.0)
        assertEquals(10.0, anomaly.rate(NetworkFeeSpeedTier.ECONOMY), 0.0)
        assertEquals(10.0, anomaly.rate(NetworkFeeSpeedTier.STANDARD), 0.0)
        assertEquals(20.0, anomaly.rate(NetworkFeeSpeedTier.PRIORITY), 0.0)
    }

    // HTTP sources

    private lateinit var server: MockWebServer

    @Before
    fun startServer() {
        server = MockWebServer().apply { start() }
    }

    @After
    fun stopServer() {
        server.shutdown()
    }

    private fun enqueue(body: String, code: Int = 200) {
        server.enqueue(MockResponse().setResponseCode(code).setBody(body))
    }

    private val baseUrl
        get() = server.url("/api").toString()

    private fun assertFetchFails(source: FeeRateSource) = runBlocking {
        try {
            source.fetch()
            fail("expected fetch to throw")
        } catch (_: Exception) {}
    }

    @Test
    fun `esplora parses primary block targets`() = runBlocking {
        enqueue("""{"1": 20.5, "2": 99, "3": 10, "4": 99, "6": 5, "144": 1.5, "25": 99}""")
        val fees = EsploraFeeSource(baseUrl).fetch()
        assertEquals("/api/fee-estimates", server.takeRequest().path)
        assertEquals(RecommendedFees(20.5, 10.0, 5.0, minimumFee = 1.5), fees)
        assertNull(fees.economyFee)
    }

    @Test
    fun `esplora falls back to targets 2 4 and 25`() = runBlocking {
        enqueue("""{"2": 18, "4": 9, "6": 5, "25": 2, "1008": 1.2}""")
        assertEquals(
            RecommendedFees(18.0, 9.0, 5.0, minimumFee = 2.0),
            EsploraFeeSource(baseUrl).fetch(),
        )
    }

    @Test
    fun `esplora falls back to target 1008 then constant minimum`() = runBlocking {
        enqueue("""{"2": 18, "4": 9, "6": 5, "1008": 1.2}""")
        assertEquals(1.2, EsploraFeeSource(baseUrl).fetch().minimumFee, 0.0)
        enqueue("""{"2": 18, "4": 9, "6": 5}""")
        assertEquals(
            Constants.MIN_FEE_RATE_SAT_VB,
            EsploraFeeSource(baseUrl).fetch().minimumFee,
            0.0,
        )
    }

    @Test
    fun `esplora missing required target throws`() {
        enqueue("""{"1": 20, "3": 10, "144": 1}""")
        assertFetchFails(EsploraFeeSource(baseUrl))
    }

    @Test
    fun `esplora non 200 throws`() {
        enqueue("""{"1": 20, "3": 10, "6": 5}""", code = 503)
        assertFetchFails(EsploraFeeSource(baseUrl))
    }

    @Test
    fun `esplora malformed body throws`() {
        enqueue("not json")
        assertFetchFails(EsploraFeeSource(baseUrl))
        enqueue("[1, 2]")
        assertFetchFails(EsploraFeeSource(baseUrl))
    }

    @Test
    fun `mempool parses recommended fees with economy`() = runBlocking {
        enqueue(
            """{"fastestFee": 25, "halfHourFee": 15, "hourFee": 8, "economyFee": 4, "minimumFee": 1}"""
        )
        val fees = MempoolFeeSource(baseUrl).fetch()
        assertEquals("/api/v1/fees/recommended", server.takeRequest().path)
        assertEquals(RecommendedFees(25.0, 15.0, 8.0, 4.0, 1.0), fees)
    }

    @Test
    fun `mempool economy fee is optional`() = runBlocking {
        enqueue("""{"fastestFee": 25, "halfHourFee": 15, "hourFee": 8, "minimumFee": 1}""")
        assertNull(MempoolFeeSource(baseUrl).fetch().economyFee)
    }

    @Test
    fun `mempool body is sanitized on ingress`() = runBlocking {
        enqueue(
            """{"fastestFee": 1000000.0, "halfHourFee": -20.0, "hourFee": 0.0, "economyFee": 1e30, "minimumFee": 1.0}"""
        )
        val fees = MempoolFeeSource(baseUrl).fetch()
        assertEquals(Constants.MAX_FEE_RATE_SAT_VB, fees.fastestFee, 0.0)
        assertEquals(1.0, fees.halfHourFee, 0.0)
        assertEquals(1.0, fees.hourFee, 0.0)
        assertEquals(Constants.MAX_FEE_RATE_SAT_VB, fees.economyFee!!, 0.0)
        assertEquals(1.0, fees.minimumFee, 0.0)
    }

    @Test
    fun `mempool missing minimum fee throws`() {
        enqueue("""{"fastestFee": 25, "halfHourFee": 15, "hourFee": 8}""")
        assertFetchFails(MempoolFeeSource(baseUrl))
    }

    @Test
    fun `mempool non 200 throws`() {
        enqueue(
            """{"fastestFee": 25, "halfHourFee": 15, "hourFee": 8, "minimumFee": 1}""",
            code = 500,
        )
        assertFetchFails(MempoolFeeSource(baseUrl))
    }

    @Test
    fun `mempool malformed body throws`() {
        enqueue("{")
        assertFetchFails(MempoolFeeSource(baseUrl))
    }

    @Test
    fun `fetch left behind by a cancelled caller is not reused later`() = runTest {
        var rate = 5.0
        val slow = FeeRateSource {
            fetchCount++
            delay(100)
            fees(rate)
        }
        val service = service(slow)

        val abandoned = launch { service.recommendedFees() }
        runCurrent()
        abandoned.cancel()
        advanceTimeBy(200)
        runCurrent()

        rate = 9.0
        assertEquals(9.0, service.recommendedFees().halfHourFee, 0.0)
        assertEquals(2, fetchCount)
    }

    @Test
    fun `esplora ignores rates sent as strings`() = runBlocking {
        enqueue("""{"1": "20", "2": 18, "3": 10, "6": 5}""")
        val fees = EsploraFeeSource(baseUrl).fetch()
        assertEquals(18.0, fees.fastestFee, 0.0)
    }
}
