package com.stablechannels.app.services.lnurl

import com.stablechannels.app.domain.lnurl.HostAddressPolicy
import com.stablechannels.app.domain.lnurl.LnurlEndpoint
import com.stablechannels.app.domain.lnurl.LnurlError
import java.io.IOException
import java.net.InetAddress
import java.net.Proxy
import java.net.URI
import java.net.UnknownHostException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.withTimeout
import okhttp3.Dns
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import okhttp3.Request

/** A resolved name that points at an address the wallet must not contact. */
class BlockedHostException(hostname: String) : UnknownHostException("Blocked address for $hostname")

/**
 * Resolves names and refuses the whole lookup when it fails, is empty, or any address is in a range
 * [HostAddressPolicy] blocks. OkHttp connects only to the addresses returned here, so the check and
 * the connection see the same addresses and DNS rebinding cannot slip in between.
 */
class SafeDns(private val delegate: Dns = Dns.SYSTEM) : Dns {
    override fun lookup(hostname: String): List<InetAddress> {
        val addresses = delegate.lookup(hostname)
        if (
            addresses.isEmpty() || addresses.any { HostAddressPolicy.isBlockedAddress(it.address) }
        ) {
            throw BlockedHostException(hostname)
        }
        return addresses
    }
}

/** Status and body of an LNURL HTTP response. */
class LnurlHttpResponse(val code: Int, val body: String)

/** Performs LNURL GET requests; [LnurlHttpClient] in the app, a fake in tests. */
fun interface LnurlTransport {
    suspend fun get(url: URI): LnurlHttpResponse
}

/**
 * GET requests to LNURL endpoints. Every URL, including each redirect target, must pass
 * [LnurlEndpoint.isAllowed]; redirects are followed here, not by OkHttp, so each hop is checked.
 * There is no proxy, so name resolution always goes through [SafeDns].
 */
class LnurlHttpClient(
    private val client: OkHttpClient = defaultClient(),
    private val requestDeadlineMillis: Long = 30_000,
    /** Which URLs may be requested; tests widen it to reach a local plain-HTTP server. */
    private val isAllowed: (URI) -> Boolean = LnurlEndpoint::isAllowed,
) : LnurlTransport {
    override suspend fun get(url: URI): LnurlHttpResponse =
        try {
            val deadlineNanos = System.nanoTime() + requestDeadlineMillis * 1_000_000
            withTimeout(requestDeadlineMillis) {
                getFollowingRedirects(url, hop = 0, deadlineNanos)
            }
        } catch (_: TimeoutCancellationException) {
            throw LnurlError.Network("Request timed out.")
        }

    private suspend fun getFollowingRedirects(
        url: URI,
        hop: Int,
        deadlineNanos: Long,
    ): LnurlHttpResponse {
        // Check the URL exactly as OkHttp will use it: its host canonicalization can turn a name
        // into an IP literal, which OkHttp connects to without consulting SafeDns.
        val httpUrl = url.toString().toHttpUrlOrNull() ?: throw LnurlError.InvalidTarget
        val canonical = httpUrl.toUri()
        if (!isAllowed(url) || !isAllowed(canonical)) throw LnurlError.InsecureEndpoint
        if (LnurlEndpoint.isOnion(url) || LnurlEndpoint.isOnion(canonical))
            throw LnurlError.OnionUnsupported
        val request =
            Request.Builder()
                .url(httpUrl)
                .header("Accept", "application/json")
                .header("User-Agent", "StableChannels")
                .build()

        val (code, location, body) =
            try {
                // A blocking socket read ignores coroutine cancellation, so OkHttp itself enforces
                // what is left of the deadline and cancels the call when it passes.
                val call = client.newCall(request)
                val remainingNanos = (deadlineNanos - System.nanoTime()).coerceAtLeast(1)
                call.timeout().timeout(remainingNanos, TimeUnit.NANOSECONDS)
                runInterruptible(Dispatchers.IO) {
                    call.execute().use { response ->
                        val source = response.body?.source()
                        val tooLarge = source != null && source.request(MAX_RESPONSE_BYTES + 1)
                        if (tooLarge) throw LnurlError.InvalidResponse
                        Triple(
                            response.code,
                            response.header("Location"),
                            source?.readUtf8().orEmpty(),
                        )
                    }
                }
            } catch (e: BlockedHostException) {
                throw LnurlError.InsecureEndpoint
            } catch (e: IOException) {
                // An interrupted read after the caller cancelled is a cancellation, not a network
                // error.
                currentCoroutineContext().ensureActive()
                throw LnurlError.Network(e.message ?: e.javaClass.simpleName)
            }

        if (code in REDIRECT_CODES) {
            val target = LnurlEndpoint.redirectTarget(canonical, location, hop, isAllowed)
            return getFollowingRedirects(target, hop + 1, deadlineNanos)
        }
        return LnurlHttpResponse(code, body)
    }

    companion object {
        const val MAX_RESPONSE_BYTES = 2L * 1024 * 1024
        private val REDIRECT_CODES = setOf(301, 302, 303, 307, 308)

        fun defaultClient(dns: Dns = SafeDns()): OkHttpClient =
            OkHttpClient.Builder()
                .dns(dns)
                .proxy(Proxy.NO_PROXY)
                .followRedirects(false)
                .followSslRedirects(false)
                .connectTimeout(15, TimeUnit.SECONDS)
                .readTimeout(15, TimeUnit.SECONDS)
                .build()
    }
}
