package com.stablechannels.app.services.lnurl

import com.stablechannels.app.domain.lnurl.LnurlEndpoint
import com.stablechannels.app.domain.lnurl.LnurlError
import java.net.InetAddress
import java.net.URI
import java.net.UnknownHostException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import okhttp3.Headers
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * Hosts that OkHttp would turn into IP literals (and connect to without consulting SafeDns) must be
 * refused before any connection, whether requested directly or reached by a redirect.
 */
class LnurlHostBypassTest {
    private val server = MockWebServer()
    private val loopback = InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1))
    private val lookups = mutableListOf<String>()

    private val testDns = fakeDns { host ->
        lookups += host
        if (host == TEST_HOST) listOf(loopback) else throw UnknownHostException(host)
    }

    /** Production policy, plus the plain-HTTP test host that serves the redirect. */
    private val client =
        LnurlHttpClient(LnurlHttpClient.defaultClient(testDns), 5_000) {
            it.host == TEST_HOST || LnurlEndpoint.isAllowed(it)
        }

    private inline fun <reified T : Throwable> assertFails(noinline block: suspend () -> Unit): T =
        assertThrows(T::class.java) { runBlocking { block() } }

    @Before fun setUp() = server.start(loopback, 0)

    @After fun tearDown() = server.shutdown()

    @Test
    fun `hosts that canonicalize to ip literals are refused without a lookup`() {
        for (url in BYPASS_URLS) {
            assertFails<LnurlError.InsecureEndpoint> {
                LnurlHttpClient(LnurlHttpClient.defaultClient(testDns)).get(URI(url))
            }
        }
        assertTrue(lookups.isEmpty())
    }

    @Test
    fun `redirects to hosts that canonicalize to ip literals are refused`() {
        for (url in BYPASS_URLS) {
            server.enqueue(
                MockResponse()
                    .setResponseCode(302)
                    .setHeaders(Headers.Builder().addUnsafeNonAscii("Location", url).build())
                    .setBody("") // restores the Content-Length that setHeaders cleared
            )
            assertFails<LnurlError.InsecureEndpoint> {
                client.get(URI("http://$TEST_HOST:${server.port}/start"))
            }
        }
        assertEquals(BYPASS_URLS.size, server.requestCount)
    }

    @Test
    fun `credentials are refused even on a host java URI cannot parse`() {
        assertFails<LnurlError.InsecureEndpoint> {
            LnurlHttpClient(LnurlHttpClient.defaultClient(testDns))
                .get(URI("https://u:p@my_host.example.com/"))
        }
        assertFails<LnurlError.InsecureEndpoint> {
            LnurlHttpClient(LnurlHttpClient.defaultClient(testDns))
                .get(URI("https://u@my_host.example.com/"))
        }
        assertTrue(lookups.isEmpty())
    }

    @Test
    fun `the url is checked again in the form OkHttp will dial`() {
        // This policy accepts the URL as written but not OkHttp's canonical form, which lowercases
        // the host. Only the second check refuses it.
        val writtenFormOnly =
            LnurlHttpClient(LnurlHttpClient.defaultClient(testDns), 5_000) {
                it.host == "PAY.EXAMPLE.COM"
            }
        assertFails<LnurlError.InsecureEndpoint> {
            writtenFormOnly.get(URI("https://PAY.EXAMPLE.COM/lnurlp"))
        }
        assertTrue(lookups.isEmpty())
    }

    @Test
    fun `cancelling the caller cancels instead of reporting a network error`() = runBlocking {
        server.enqueue(
            MockResponse().setBody("late").setHeadersDelay(5, java.util.concurrent.TimeUnit.SECONDS)
        )
        var reported: Throwable? = null
        val job = launch {
            try {
                client.get(URI("http://$TEST_HOST:${server.port}/slow"))
            } catch (e: LnurlError) {
                reported = e
            } catch (e: CancellationException) {
                throw e
            }
        }
        delay(300)
        job.cancelAndJoin()
        assertTrue(job.isCancelled)
        assertNull(reported)
    }

    private companion object {
        const val TEST_HOST = "pay.example.test"
        val BYPASS_URLS =
            listOf(
                "https://１２７.０.０.１/x", // full-width digits
                "https://127。0。0。1/x", // ideographic full stop
                "https://１０.0.0.1/x",
                "https://2130706433/x",
                "https://127.1/x",
                "https://10.1/x",
                "https://x.ｏｎｉｏｎ/x", // full-width .onion
            )
    }
}
