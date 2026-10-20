package com.stablechannels.app.domain.lnurl

import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class LnurlEndpointTest {

    private fun errorOf(block: () -> Unit): LnurlError? =
        try {
            block()
            null
        } catch (e: LnurlError) {
            e
        }

    private fun assertInvalidTarget(input: String) {
        assertEquals(
            "Expected InvalidTarget for $input",
            LnurlError.InvalidTarget,
            errorOf { LnurlEndpoint.resolve(input) },
        )
    }

    private val bech32Sample =
        "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"

    @Test
    fun `resolve Lightning Address endpoint`() {
        assertEquals(
            "https://bitcoin.org/.well-known/lnurlp/satoshi",
            LnurlEndpoint.resolve("satoshi@bitcoin.org").toString(),
        )
        assertEquals(
            "https://0xprabal.com/.well-known/lnurlp/prabal",
            LnurlEndpoint.resolve("prabal@0xprabal.com").toString(),
        )
        assertEquals(
            "https://bitcoin.org/.well-known/lnurlp/satoshi",
            LnurlEndpoint.resolve("lightning:satoshi@bitcoin.org").toString(),
        )
        assertEquals(
            "https://bitcoin.org/.well-known/lnurlp/satoshi",
            LnurlEndpoint.resolve("lightning://satoshi@bitcoin.org").toString(),
        )
        assertEquals(
            "https://service.com/api/v1/lnurl/pay",
            LnurlEndpoint.resolve(bech32Sample).toString(),
        )
        assertEquals(
            "https://service.com/api/v1/lnurl/pay",
            LnurlEndpoint.resolve("lightning://$bech32Sample").toString(),
        )

        assertInvalidTarget("invalid-target")
        assertInvalidTarget("user@@domain.com")
        assertInvalidTarget("user@nodomain")
        assertInvalidTarget("http://insecure-clearnet.com")

        assertEquals(
            "http://xyz.onion/.well-known/lnurlp/anon",
            LnurlEndpoint.resolve("anon@xyz.onion").toString(),
        )
        assertEquals(
            "http://xyz.onion/api/lnurlp",
            LnurlEndpoint.resolve("http://xyz.onion/api/lnurlp").toString(),
        )
    }

    @Test
    fun `resolve rejects private and loopback hosts`() {
        listOf(
                "alice@127.0.0.1",
                "bob@localhost",
                "carol@192.168.1.1",
                "david@10.0.0.1",
                "eve@169.254.169.254",
                "frank@mydevice.local",
                "https://127.0.0.1/lnurlp",
                "https://localhost/lnurlp",
                "https://192.168.1.1/.well-known/lnurlp/alice",
                "http://127.0.0.1/lnurlp",
                "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excuq5ex0g8", // localhost
                "lnurl1dp68gurn8ghj7vfjxuhrqt3s9ccj7mrww4excuqw06qrc", // 127.0.0.1
                "lnurl1dp68gurn8ghj7vfs9cczuvpwxyhkcmn4wfk8qdmhwmg", // 10.0.0.1
                "lnurl1dp68gurn8ghj7vfk8yhrydf59ccnvwfwxg6ngtmvde6hymrsn3sg3p", // 169.254.169.254
                "lnurl1dp68gurn8ghj7vfexghrzd3c9ccjuvf0d3h82unvwq0hrk4j", // 192.168.1.1
                "lnurl1dp68gurn8ghj7mtev3jhv6trv5hxcmmrv9kz7mrww4excuq6q74wt", // mydevice.local
            )
            .forEach(::assertInvalidTarget)
    }

    @Test
    fun `resolve rejects non-ASCII homograph domain`() {
        assertInvalidTarget("alice@exаmple.com")
    }

    @Test
    fun `resolve rejects oversized and zero port`() {
        assertInvalidTarget("https://example.com:99999/pay")
        assertInvalidTarget("https://example.com:0/pay")
    }

    @Test
    fun `isAllowed static checks`() {
        assertTrue(LnurlEndpoint.isAllowed(URI("https://service.example.com/api?foo=bar")))
        assertTrue(LnurlEndpoint.isAllowed(URI("https://service.example.com:8443/api")))
        assertTrue(LnurlEndpoint.isAllowed(URI("http://service.onion/api")))
        // Clearnet IP literals are refused, public or not.
        assertFalse(LnurlEndpoint.isAllowed(URI("https://8.8.8.8/api")))
        assertFalse(LnurlEndpoint.isAllowed(URI("https://127.0.0.1/api")))
        assertFalse(LnurlEndpoint.isAllowed(URI("https://[2606:4700::1111]/api")))
        assertFalse(LnurlEndpoint.isAllowed(URI("http://service.example.com/api")))
    }

    @Test
    fun `isAllowed rejects non-canonical IPv4 literal URLs`() {
        for (url in
            listOf(
                "https://0127.0.0.1/api",
                "https://00127.0.0.1/api",
                "https://010.0.0.1/api",
                "https://0172.016.0.1/api",
                "https://0100.064.0.1/api",
                "https://169.0254.169.254/api",
                "https://0177.0.0.1/api",
            )) {
            assertFalse("Expected $url blocked", LnurlEndpoint.isAllowed(URI(url)))
        }
    }

    @Test
    fun `isAllowed rejects userinfo`() {
        assertFalse(LnurlEndpoint.isAllowed(URI("https://user:password@auth.example.com/api")))
    }

    @Test
    fun `isAllowed rejects trailing-dot local hosts`() {
        assertFalse(LnurlEndpoint.isAllowed(URI("https://localhost./pay")))
        assertFalse(LnurlEndpoint.isAllowed(URI("https://server.internal./pay")))
        assertFalse(LnurlEndpoint.isAllowed(URI("https://gateway.local./pay")))
    }

    @Test
    fun `hostOf reads underscore host that URI leaves unparsed`() {
        val url = URI("https://my_host.example.com/pay")
        assertEquals(null, url.host)
        assertEquals("my_host.example.com", LnurlEndpoint.hostOf(url))
        assertEquals(
            "my_host.example.com",
            LnurlEndpoint.hostOf(URI("https://u@my_host.example.com:8443/pay")),
        )
        assertEquals("example.com", LnurlEndpoint.hostOf(URI("https://example.com/pay")))
    }

    @Test
    fun `redirect within hop limit resolves relative location`() {
        val base = URI("https://example.com/api/v1/pay")
        assertEquals(
            "https://example.com/api/v2/pay",
            LnurlEndpoint.redirectTarget(base, "/api/v2/pay", hop = 0).toString(),
        )
        assertEquals(
            "https://example.com/api/v2/pay",
            LnurlEndpoint.redirectTarget(base, "/api/v2/pay", hop = LnurlEndpoint.MAX_REDIRECTS - 1)
                .toString(),
        )
    }

    @Test
    fun `redirect beyond hop limit fails`() {
        val base = URI("https://example.com/api/v1/pay")
        assertEquals(
            LnurlError.TooManyRedirects,
            errorOf { LnurlEndpoint.redirectTarget(base, "/api/v2/pay", hop = 3) },
        )
    }

    @Test
    fun `redirect without location is invalid response`() {
        val base = URI("https://example.com/api/v1/pay")
        assertEquals(
            LnurlError.InvalidResponse,
            errorOf { LnurlEndpoint.redirectTarget(base, null, hop = 0) },
        )
    }

    @Test
    fun `redirect from clearnet to onion is vetoed`() {
        assertEquals(
            LnurlError.InsecureEndpoint,
            errorOf {
                LnurlEndpoint.redirectTarget(
                    URI("https://example.com/pay"),
                    "http://service.onion/pay",
                    hop = 0,
                )
            },
        )
    }

    @Test
    fun `redirect from https to http is vetoed`() {
        assertEquals(
            LnurlError.InsecureEndpoint,
            errorOf {
                LnurlEndpoint.redirectTarget(
                    URI("https://example.com/pay"),
                    "http://example.com/pay",
                    hop = 0,
                )
            },
        )
    }

    @Test
    fun `redirect to private IP is vetoed`() {
        assertEquals(
            LnurlError.InsecureEndpoint,
            errorOf {
                LnurlEndpoint.redirectTarget(
                    URI("https://example.com/pay"),
                    "https://10.0.0.1/pay",
                    hop = 0,
                )
            },
        )
    }

    @Test
    fun `redirect uses injected allow check`() {
        try {
            LnurlEndpoint.redirectTarget(
                URI("https://example.com/pay"),
                "https://internal.lan/pay",
                hop = 0,
                allow = { false },
            )
            fail("Expected InsecureEndpoint")
        } catch (e: LnurlError) {
            assertEquals(LnurlError.InsecureEndpoint, e)
        }
    }
}
