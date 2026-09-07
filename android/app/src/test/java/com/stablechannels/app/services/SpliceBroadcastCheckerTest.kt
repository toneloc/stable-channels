package com.stablechannels.app.services

import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.SocketPolicy
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import java.util.concurrent.TimeUnit

class SpliceBroadcastCheckerTest {

    private val servers = mutableListOf<MockWebServer>()
    private lateinit var httpClient: OkHttpClient

    @Before
    fun setUp() {
        httpClient = OkHttpClient.Builder()
            .connectTimeout(500, TimeUnit.MILLISECONDS)
            .readTimeout(500, TimeUnit.MILLISECONDS)
            .callTimeout(1, TimeUnit.SECONDS)
            .build()
    }

    @After
    fun tearDown() {
        servers.forEach { it.shutdown() }
        servers.clear()
    }

    private fun newServer(): MockWebServer = MockWebServer().also { it.start(); servers.add(it) }

    private fun checker(retries: Int = 3, retryDelayMs: Long = 0L) =
        SpliceBroadcastChecker(httpClient, retries = retries, retryDelayMs = retryDelayMs, sleep = {}, logWarning = {})

    @Test
    fun `single endpoint 200 returns EXISTS`() {
        val server = newServer()
        server.enqueue(MockResponse().setResponseCode(200).setBody("{\"confirmed\":true}"))

        val result = checker().checkStatus("abc", listOf(server.url("/").toString()))

        assertEquals(TxBroadcastStatus.EXISTS, result)
    }

    @Test
    fun `consistent 404 across retries returns NOT_FOUND`() {
        val server = newServer()
        repeat(3) { server.enqueue(MockResponse().setResponseCode(404)) }

        val result = checker(retries = 3).checkStatus("abc", listOf(server.url("/").toString()))

        assertEquals(TxBroadcastStatus.NOT_FOUND, result)
    }

    @Test
    fun `single 429 response is INCONCLUSIVE not a failure verdict`() {
        val server = newServer()
        server.enqueue(MockResponse().setResponseCode(429))

        val result = checker().checkStatus("abc", listOf(server.url("/").toString()))

        assertEquals(TxBroadcastStatus.INCONCLUSIVE, result)
    }

    @Test
    fun `single 500 response is INCONCLUSIVE not a failure verdict`() {
        val server = newServer()
        server.enqueue(MockResponse().setResponseCode(500))

        val result = checker().checkStatus("abc", listOf(server.url("/").toString()))

        assertEquals(TxBroadcastStatus.INCONCLUSIVE, result)
    }

    @Test
    fun `socket timeout is INCONCLUSIVE not a failure verdict`() {
        val server = newServer()
        server.enqueue(MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE))

        val result = checker().checkStatus("abc", listOf(server.url("/").toString()))

        assertEquals(TxBroadcastStatus.INCONCLUSIVE, result)
    }

    @Test
    fun `mixed endpoints one 200 one 404 returns EXISTS`() {
        val existsServer = newServer()
        existsServer.enqueue(MockResponse().setResponseCode(200))
        val notFoundServer = newServer()
        notFoundServer.enqueue(MockResponse().setResponseCode(404))

        val result = checker().checkStatus(
            "abc",
            listOf(notFoundServer.url("/").toString(), existsServer.url("/").toString())
        )

        assertEquals(TxBroadcastStatus.EXISTS, result)
    }

    @Test
    fun `mixed endpoints one 404 one 500 is INCONCLUSIVE`() {
        val notFoundServer = newServer()
        notFoundServer.enqueue(MockResponse().setResponseCode(404))
        val errorServer = newServer()
        errorServer.enqueue(MockResponse().setResponseCode(500))

        val result = checker().checkStatus(
            "abc",
            listOf(notFoundServer.url("/").toString(), errorServer.url("/").toString())
        )

        assertEquals(TxBroadcastStatus.INCONCLUSIVE, result)
    }

    @Test
    fun `tx propagates between retry rounds resolves to EXISTS`() {
        // First round: not found yet. Second round: tx has since propagated.
        val server = newServer()
        server.enqueue(MockResponse().setResponseCode(404))
        server.enqueue(MockResponse().setResponseCode(200))

        val result = checker(retries = 2).checkStatus("abc", listOf(server.url("/").toString()))

        assertEquals(TxBroadcastStatus.EXISTS, result)
    }

    @Test
    fun `empty endpoint list is INCONCLUSIVE`() {
        val result = checker().checkStatus("abc", emptyList())

        assertEquals(TxBroadcastStatus.INCONCLUSIVE, result)
    }
}
