package com.stablechannels.app.services.lnurl

import com.stablechannels.app.domain.lnurl.LnurlError
import java.net.InetAddress
import java.net.URI
import java.net.UnknownHostException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import okhttp3.Dns
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Before
import org.junit.Test

class LnurlHttpClientTest {

    private val server = MockWebServer()
    private val loopback = InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1))

    /** Test names resolve to the local server without the private-address check. */
    private val testDns = fakeDns { host ->
        if (host in ALLOWED_HOSTS) listOf(loopback) else throw UnknownHostException(host)
    }

    private val allowTestHosts: (URI) -> Boolean = { it.host in ALLOWED_HOSTS }

    private fun client(dns: Dns = testDns, deadlineMillis: Long = 30_000) =
        LnurlHttpClient(LnurlHttpClient.defaultClient(dns), deadlineMillis, allowTestHosts)

    private fun url(path: String, host: String = "pay.example.test") =
        URI("http://$host:${server.port}$path")

    private fun redirect(location: String, code: Int = 302) =
        MockResponse().setResponseCode(code).setHeader("Location", location)

    private inline fun <reified T : Throwable> assertFails(noinline block: suspend () -> Unit): T =
        assertThrows(T::class.java) { runBlocking { block() } }

    @Before fun setUp() = server.start(loopback, 0)

    @After fun tearDown() = server.shutdown()

    @Test
    fun `200 body is returned`() = runBlocking {
        server.enqueue(MockResponse().setBody("""{"tag":"payRequest"}"""))
        val response = client().get(url("/.well-known/lnurlp/alice"))
        assertEquals(200, response.code)
        assertEquals("""{"tag":"payRequest"}""", response.body)
        val request = server.takeRequest()
        assertEquals("/.well-known/lnurlp/alice", request.path)
        assertEquals("application/json", request.getHeader("Accept"))
    }

    @Test
    fun `non 2xx status is returned for the service to classify`() = runBlocking {
        server.enqueue(MockResponse().setResponseCode(404).setBody("missing"))
        val response = client().get(url("/x"))
        assertEquals(404, response.code)
        assertEquals("missing", response.body)
    }

    @Test
    fun `301 to an allowed path is followed`() = runBlocking {
        server.enqueue(redirect("/next", code = 301))
        server.enqueue(MockResponse().setBody("ok"))
        val response = client().get(url("/start"))
        assertEquals(200, response.code)
        assertEquals("ok", response.body)
        assertEquals("/start", server.takeRequest().path)
        assertEquals("/next", server.takeRequest().path)
    }

    @Test
    fun `redirect to another allowed host is followed`() = runBlocking {
        server.enqueue(redirect(url("/cb", host = "other.example.test").toString()))
        server.enqueue(MockResponse().setBody("ok"))
        assertEquals("ok", client().get(url("/start")).body)
        server.takeRequest()
        assertEquals("other.example.test:${server.port}", server.takeRequest().getHeader("Host"))
    }

    @Test
    fun `redirect to a disallowed host is InsecureEndpoint and never contacted`() {
        server.enqueue(redirect(url("/insecure-callback", host = "service.example.com").toString()))
        server.enqueue(MockResponse().setBody("should not be served"))
        assertFails<LnurlError.InsecureEndpoint> { client().get(url("/callback")) }
        assertEquals(1, server.requestCount)
    }

    @Test
    fun `redirect to a private ip literal is InsecureEndpoint`() {
        server.enqueue(redirect("https://127.0.0.1/callback"))
        assertFails<LnurlError.InsecureEndpoint> { client().get(url("/callback")) }
        assertEquals(1, server.requestCount)
    }

    @Test
    fun `redirect to a name resolving to a private address is InsecureEndpoint`() {
        val private = InetAddress.getByAddress(byteArrayOf(192.toByte(), 168.toByte(), 1, 50))
        val guarded = SafeDns(fakeDns { listOf(private) })
        val dns = fakeDns { host ->
            if (host == "pay.example.test") listOf(loopback) else guarded.lookup(host)
        }
        server.enqueue(redirect(url("/internal", host = "other.example.test").toString()))
        assertFails<LnurlError.InsecureEndpoint> { client(dns).get(url("/callback")) }
        assertEquals(1, server.requestCount)
    }

    @Test
    fun `three redirects are followed`() = runBlocking {
        repeat(3) { server.enqueue(redirect("/hop$it")) }
        server.enqueue(MockResponse().setBody("ok"))
        assertEquals("ok", client().get(url("/start")).body)
        assertEquals(4, server.requestCount)
    }

    @Test
    fun `four chained redirects is TooManyRedirects`() {
        repeat(4) { server.enqueue(redirect("/hop$it")) }
        server.enqueue(MockResponse().setBody("should not be served"))
        assertFails<LnurlError.TooManyRedirects> { client().get(url("/start")) }
        assertEquals(4, server.requestCount)
    }

    @Test
    fun `redirect without Location is InvalidResponse`() {
        server.enqueue(MockResponse().setResponseCode(302))
        assertFails<LnurlError.InvalidResponse> { client().get(url("/start")) }
    }

    @Test
    fun `body of exactly 2 MiB is accepted`() = runBlocking {
        val max = LnurlHttpClient.MAX_RESPONSE_BYTES.toInt()
        server.enqueue(MockResponse().setBody("a".repeat(max)))
        assertEquals(max, client().get(url("/big")).body.length)
    }

    @Test
    fun `body over 2 MiB is InvalidResponse`() {
        val max = LnurlHttpClient.MAX_RESPONSE_BYTES.toInt()
        server.enqueue(MockResponse().setBody("a".repeat(max + 1)))
        assertFails<LnurlError.InvalidResponse> { client().get(url("/big")) }
    }

    @Test
    fun `slow response past the deadline is Network`() {
        server.enqueue(MockResponse().setBody("late").setHeadersDelay(5, TimeUnit.SECONDS))
        val started = System.nanoTime()
        assertFails<LnurlError.Network> { client(deadlineMillis = 300).get(url("/slow")) }
        val elapsedMillis = (System.nanoTime() - started) / 1_000_000
        assert(elapsedMillis < 4_000) { "deadline not enforced: $elapsedMillis ms" }
    }

    @Test
    fun `SafeDns refuses a name resolving to loopback before connecting`() {
        val dns = SafeDns(fakeDns { listOf(loopback) })
        assertFails<LnurlError.InsecureEndpoint> { client(dns).get(url("/x")) }
        assertEquals(0, server.requestCount)
    }

    // Fails closed: no request is made, though as Network rather than InsecureEndpoint.
    @Test
    fun `dns failure fails closed`() {
        val dns = SafeDns(fakeDns { throw UnknownHostException(it) })
        assertFails<LnurlError.Network> { client(dns).get(url("/x")) }
        assertEquals(0, server.requestCount)
    }

    @Test
    fun `empty dns answer is InsecureEndpoint`() {
        val dns = SafeDns(fakeDns { emptyList() })
        assertFails<LnurlError.InsecureEndpoint> { client(dns).get(url("/x")) }
        assertEquals(0, server.requestCount)
    }

    @Test
    fun `onion url is OnionUnsupported without any request`() {
        var calls = 0
        val recording =
            OkHttpClient.Builder()
                .addInterceptor {
                    calls++
                    it.proceed(it.request())
                }
                .build()
        assertFails<LnurlError.OnionUnsupported> {
            LnurlHttpClient(recording).get(URI("http://service.onion/lnurlp"))
        }
        assertEquals(0, calls)
    }

    @Test
    fun `clearnet http is InsecureEndpoint under the default policy`() {
        assertFails<LnurlError.InsecureEndpoint> {
            LnurlHttpClient(LnurlHttpClient.defaultClient(testDns)).get(url("/x"))
        }
        assertEquals(0, server.requestCount)
    }

    private companion object {
        val ALLOWED_HOSTS = setOf("pay.example.test", "other.example.test")
    }
}
