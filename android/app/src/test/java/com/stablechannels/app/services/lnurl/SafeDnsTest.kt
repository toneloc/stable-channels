package com.stablechannels.app.services.lnurl

import java.net.InetAddress
import java.net.UnknownHostException
import okhttp3.Dns
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class SafeDnsTest {

    private fun ip(vararg bytes: Int): InetAddress =
        InetAddress.getByAddress("host.example.test", ByteArray(bytes.size) { bytes[it].toByte() })

    private fun v6(vararg words: Int): InetAddress =
        ip(*words.flatMap { listOf(it shr 8, it and 0xFF) }.toIntArray())

    private val public4 = ip(93, 184, 216, 34)
    private val public6 = v6(0x2606, 0x4700, 0, 0, 0, 0, 0, 0x1111)

    private fun dnsReturning(vararg addresses: InetAddress) =
        SafeDns(fakeDns { addresses.toList() })

    @Test
    fun `public addresses are returned unchanged`() {
        val addresses = listOf(public4, public6)
        assertEquals(addresses, SafeDns(fakeDns { addresses }).lookup("pay.example.com"))
    }

    @Test
    fun `any private address rejects the whole lookup`() {
        val blocked =
            listOf(
                ip(10, 0, 0, 1),
                ip(127, 0, 0, 1),
                ip(192, 168, 1, 50),
                ip(169, 254, 169, 254),
                v6(0, 0, 0, 0, 0, 0, 0, 1), // ::1
                v6(0, 0, 0, 0, 0, 0xFFFF, 0xC0A8, 0x0101), // ::ffff:192.168.1.1
                v6(0xFD00, 0, 0, 0, 0, 0, 0, 1), // fd00::1
            )
        for (address in blocked) {
            assertThrows(BlockedHostException::class.java) {
                dnsReturning(address).lookup("pay.example.com")
            }
            // Mixed with a public address it is still refused.
            assertThrows(BlockedHostException::class.java) {
                dnsReturning(public4, address).lookup("pay.example.com")
            }
        }
    }

    @Test
    fun `empty result is refused`() {
        assertThrows(BlockedHostException::class.java) { dnsReturning().lookup("pay.example.com") }
    }

    @Test
    fun `lookup failure propagates as UnknownHostException`() {
        val failing = SafeDns(fakeDns { throw UnknownHostException(it) })
        assertThrows(UnknownHostException::class.java) { failing.lookup("nonexistent.example") }
    }

    // A name that answers public first and private later is checked on every lookup.
    @Test
    fun `rebinding to a private address on a later lookup is refused`() {
        var calls = 0
        val rebinding =
            SafeDns(fakeDns { if (++calls == 1) listOf(public4) else listOf(ip(10, 0, 0, 1)) })
        assertEquals(listOf(public4), rebinding.lookup("rebinding.example.com"))
        assertThrows(BlockedHostException::class.java) { rebinding.lookup("rebinding.example.com") }
    }
}

/** okhttp3.Dns is not a fun interface; this adapts a lambda for the tests. */
internal fun fakeDns(resolve: (String) -> List<InetAddress>) =
    object : Dns {
        override fun lookup(hostname: String) = resolve(hostname)
    }
