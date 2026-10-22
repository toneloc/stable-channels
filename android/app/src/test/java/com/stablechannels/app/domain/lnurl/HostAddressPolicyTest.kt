package com.stablechannels.app.domain.lnurl

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HostAddressPolicyTest {

    private fun bytes(vararg values: Int): ByteArray =
        ByteArray(values.size) { values[it].toByte() }

    @Test
    fun `isPrivateOrLoopbackHost identifies all restricted ranges`() {
        val restricted =
            listOf(
                "localhost",
                "sub.localhost",
                "router.local",
                "cluster.internal",
                "127.0.0.1",
                "127.255.255.254",
                "10.0.0.1",
                "10.254.1.9",
                "172.16.0.1",
                "172.31.255.254",
                "192.168.0.1",
                "192.168.100.200",
                "169.254.169.254",
                "100.64.0.1",
                "100.127.255.254",
                "198.18.0.1",
                "198.19.255.254",
                "192.0.0.1",
                "192.0.2.1",
                "198.51.100.1",
                "203.0.113.1",
                "224.0.0.1",
                "239.255.255.250",
                "240.0.0.1",
                "255.255.255.255",
                "0.0.0.0",
                "::1",
                "[::1]",
                "::",
                "fe80::1",
                "fc00::1",
                "fd12:3456:789a::1",
                "ff00::1",
                "ff02::1",
                "100::1",
                "2001:db8::1",
                "2002:0a00:0001::",
                "2002:7f00:0001::",
                "2001:0000:0000:0000:0000:0000:f5ff:fffe",
                "::ffff:10.0.0.1",
                "::ffff:127.0.0.1",
                "::ffff:192.168.1.1",
            )
        for (host in restricted) {
            assertTrue(
                "Expected $host to be private/loopback",
                HostAddressPolicy.isPrivateOrLoopbackHost(host),
            )
        }

        val allowed =
            listOf(
                "bitcoin.org",
                "ln.tips",
                "example.com",
                "service.onion",
                "8.8.8.8",
                "172.15.255.255",
                "172.32.0.1",
                "100.128.0.1",
                "198.20.0.1",
                "1.1.1.1",
                "2002:5db8:d822::",
                "2606:4700:4700::1111",
            )
        for (host in allowed) {
            assertFalse(
                "Expected $host to be public",
                HostAddressPolicy.isPrivateOrLoopbackHost(host),
            )
        }
    }

    @Test
    fun `blocks all missing subnets`() {
        val blocked =
            listOf(
                "64:ff9b::a9fe:a9fe", // NAT64 169.254.169.254
                "64:ff9b::7f00:1", // NAT64 127.0.0.1
                "64:ff9b:1::1", // local-use NAT64
                "fec0::1", // deprecated site-local
                "2001:10::1", // ORCHID
                "2001:20::1", // ORCHIDv2
                "2002:c058:6301::", // 6to4 embedding 192.88.99.1
                "192.88.99.1", // 6to4 anycast relay
                "::ffff:0:127.0.0.1", // SIIT
                "::ffff:0:10.0.0.1", // SIIT
                "3fff::1", // IPv6 documentation
            )
        for (ip in blocked) {
            assertTrue("Expected $ip to be blocked", HostAddressPolicy.isPrivateOrLoopbackHost(ip))
        }
        for (ip in listOf("2606:4700::1111", "1.1.1.1", "93.184.216.34", "::ffff:0:8.8.8.8")) {
            assertFalse("Expected $ip to be allowed", HostAddressPolicy.isPrivateOrLoopbackHost(ip))
        }
    }

    @Test
    fun `non-canonical IPv4 literals are blocked or not IP literals`() {
        for (ip in
            listOf(
                "0127.0.0.1",
                "00127.0.0.1",
                "010.0.0.1",
                "0172.016.0.1",
                "0100.064.0.1",
                "169.0254.169.254",
                "0177.0.0.1",
                "127.0.0.1",
                "10.0.0.1",
            )) {
            assertEquals("Expected $ip blocked", true, HostAddressPolicy.isBlockedIpLiteral(ip))
        }
        assertEquals(false, HostAddressPolicy.isBlockedIpLiteral("8.8.8.8"))

        // Not numeric literals; a hostname with these labels is checked after DNS resolution.
        assertNull(HostAddressPolicy.isBlockedIpLiteral("0x7f.0.0.1"))
        assertNull(HostAddressPolicy.isBlockedIpLiteral("2130706433"))
        assertNull(HostAddressPolicy.isBlockedIpLiteral("127.1"))

        for (host in listOf("0127.0.0.1", "010.0.0.1", "169.0254.169.254", "0177.0.0.1")) {
            assertTrue(host, HostAddressPolicy.isPrivateOrLoopbackHost(host))
        }
    }

    @Test
    fun `canonical numeric IP inputs`() {
        // Leading-zero dotted quad is ambiguous and counts as blocked.
        assertTrue(HostAddressPolicy.hasLeadingZeroIpv4Octet("0177.0.0.1"))
        assertEquals(true, HostAddressPolicy.isBlockedIpLiteral("0177.0.0.1"))

        assertEquals(0x08080808L, HostAddressPolicy.parseIpv4("8.8.8.8"))

        assertEquals(
            HostAddressPolicy.parseIpv6("2606:4700::1111"),
            HostAddressPolicy.parseIpv6("2606:4700:0:0:0:0:0:1111"),
        )

        // Zone identifier is ambiguous and counts as blocked.
        assertEquals(true, HostAddressPolicy.isBlockedIpLiteral("fe80::1%en0"))
    }

    @Test
    fun `parseIpv4 is strict`() {
        assertEquals(0x7F000001L, HostAddressPolicy.parseIpv4("127.0.0.1"))
        assertEquals(0xFFFFFFFFL, HostAddressPolicy.parseIpv4("255.255.255.255"))
        assertNull(HostAddressPolicy.parseIpv4("256.0.0.1"))
        assertNull(HostAddressPolicy.parseIpv4("127.1"))
        assertNull(HostAddressPolicy.parseIpv4("0x7f.0.0.1"))
        assertNull(HostAddressPolicy.parseIpv4("1.2.3.4.5"))
        assertNull(HostAddressPolicy.parseIpv4("1..2.3"))
    }

    @Test
    fun `parseIpv6 expands compressed and dotted forms`() {
        assertEquals(List(16) { 0 }, HostAddressPolicy.parseIpv6("::"))
        assertEquals(List(15) { 0 } + 1, HostAddressPolicy.parseIpv6("::1"))
        assertEquals(
            List(10) { 0 } + listOf(0xFF, 0xFF, 10, 0, 0, 1),
            HostAddressPolicy.parseIpv6("::ffff:10.0.0.1"),
        )
        assertEquals(
            listOf(0x00, 0x64, 0xFF, 0x9B) + List(8) { 0 } + listOf(0x7F, 0, 0, 1),
            HostAddressPolicy.parseIpv6("64:ff9b::7f00:1"),
        )
        assertEquals(
            listOf(0x20, 0x02, 0x0A, 0x00, 0x00, 0x01) + List(10) { 0 },
            HostAddressPolicy.parseIpv6("2002:0a00:0001::"),
        )
        assertNull(HostAddressPolicy.parseIpv6("1::2::3"))
        assertNull(HostAddressPolicy.parseIpv6("12345::"))
        assertNull(HostAddressPolicy.parseIpv6("1:2:3:4:5:6:7"))
        assertNull(HostAddressPolicy.parseIpv6("g::1"))
        assertNull(HostAddressPolicy.parseIpv6("example.com"))
    }

    @Test
    fun `IPv6 literal block decisions`() {
        val blocked =
            listOf(
                "::",
                "::1",
                "::ffff:10.0.0.1",
                "64:ff9b::7f00:1",
                "2002:0a00:0001::",
                "2001:0000:4136:e378:8000:63bf:f5ff:fffe", // Teredo client 10.0.0.1
                "fe80::1",
                "fc00::1",
                "ff02::1",
                "2001:db8::1",
            )
        for (ip in blocked) {
            assertEquals("Expected $ip blocked", true, HostAddressPolicy.isBlockedIpLiteral(ip))
        }
        assertEquals(false, HostAddressPolicy.isBlockedIpLiteral("2606:4700::1111"))
        // Teredo client mapping to public 8.8.8.8 (f7f7:f7f7 inverted).
        assertEquals(
            false,
            HostAddressPolicy.isBlockedIpLiteral("2001:0:4136:e378:8000:63bf:f7f7:f7f7"),
        )
        assertNull(HostAddressPolicy.isBlockedIpLiteral("example.com"))
    }

    @Test
    fun `isBlockedAddress handles 4 and 16 byte addresses`() {
        assertTrue(HostAddressPolicy.isBlockedAddress(bytes(127, 0, 0, 1)))
        assertTrue(HostAddressPolicy.isBlockedAddress(bytes(10, 0, 0, 1)))
        assertTrue(HostAddressPolicy.isBlockedAddress(bytes(169, 254, 169, 254)))
        assertFalse(HostAddressPolicy.isBlockedAddress(bytes(93, 184, 216, 34)))
        assertFalse(HostAddressPolicy.isBlockedAddress(bytes(8, 8, 8, 8)))

        val loopback6 = ByteArray(16).also { it[15] = 1 }
        assertTrue(HostAddressPolicy.isBlockedAddress(loopback6))
        val mapped10 = bytes(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 10, 0, 0, 1)
        assertTrue(HostAddressPolicy.isBlockedAddress(mapped10))
        val public6 = bytes(0x26, 0x06, 0x47, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11)
        assertFalse(HostAddressPolicy.isBlockedAddress(public6))

        // Dual stack: both families public, so both are acceptable at connect time.
        assertFalse(HostAddressPolicy.isBlockedAddress(public6))
        assertFalse(HostAddressPolicy.isBlockedAddress(bytes(93, 184, 216, 34)))

        assertTrue(HostAddressPolicy.isBlockedAddress(ByteArray(5)))
        assertTrue(HostAddressPolicy.isBlockedAddress(ByteArray(0)))
    }

    @Test
    fun `cleanHost strips trailing dots and brackets`() {
        assertEquals("localhost", HostAddressPolicy.cleanHost("localhost."))
        assertEquals("service.internal", HostAddressPolicy.cleanHost("service.internal."))
        assertEquals("router.local", HostAddressPolicy.cleanHost("router.local."))
        assertEquals("myhost.localhost", HostAddressPolicy.cleanHost("myhost.localhost."))
        assertEquals("::1", HostAddressPolicy.cleanHost(" [::1] "))
        assertEquals("example.com", HostAddressPolicy.cleanHost("EXAMPLE.com"))
    }
}
