package com.stablechannels.app.domain.send

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class Bip21ParserTest {
    private val address = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"

    @Test
    fun `parses address amount lightning and offer`() {
        val uri = Bip21Parser.parse("bitcoin:$address?amount=0.0003&lightning=lnbc1abc&lno=lno1xyz")
        assertNotNull(uri)
        assertEquals(address, uri!!.address)
        assertEquals(30_000L, uri.amountSats)
        assertEquals("lnbc1abc", uri.lightning)
        assertEquals("lno1xyz", uri.offer)
        assertNull(uri.unhandledRequiredParameter)
        assertFalse(uri.hasDuplicateParameter)
    }

    @Test
    fun `parses bare address and double slash scheme`() {
        val bare = Bip21Parser.parse("BITCOIN:$address")!!
        assertEquals(address, bare.address)
        assertNull(bare.amountSats)
        assertNull(bare.lightning)
        assertNull(bare.offer)

        assertEquals(address, Bip21Parser.parse("bitcoin://$address?amount=1")!!.address)
    }

    @Test
    fun `req- parameter is flagged`() {
        val uri = Bip21Parser.parse("bitcoin:$address?req-somethingnew=1")!!
        assertEquals("req-somethingnew", uri.unhandledRequiredParameter)
    }

    @Test
    fun `pop parameter is flagged`() {
        val uri = Bip21Parser.parse("bitcoin:$address?pop=callback%3a")!!
        assertEquals("pop", uri.unhandledRequiredParameter)
    }

    @Test
    fun `duplicate amount is flagged`() {
        val uri = Bip21Parser.parse("bitcoin:$address?amount=1&amount=2")!!
        assertTrue(uri.hasDuplicateParameter)
    }

    @Test
    fun `values are percent decoded`() {
        val uri = Bip21Parser.parse("bitcoin:$address?lightning=lnbc%31abc&label=a%20b")!!
        assertEquals("lnbc1abc", uri.lightning)
    }

    @Test
    fun `non bitcoin input returns null`() {
        assertNull(Bip21Parser.parse(address))
        assertNull(Bip21Parser.parse("lightning:lnbc1abc"))
        assertNull(Bip21Parser.parse(""))
    }

    @Test
    fun `btcToSats rejects malformed and out of range amounts`() {
        listOf("1e3", "-1", "0", "0.000000001", "21000000.00000001").forEach {
            assertNull(it, Bip21Parser.btcToSats(it))
        }
    }

    @Test
    fun `btcToSats converts valid amounts`() {
        assertEquals(30_000L, Bip21Parser.btcToSats("0.0003"))
        assertEquals(2_100_000_000_000_000L, Bip21Parser.btcToSats("21000000"))
    }

    @Test
    fun `plus in a value is kept, not turned into a space`() {
        assertEquals("lno1aaa+bbb", Bip21Parser.parse("bitcoin:?lno=lno1aaa+bbb")!!.offer)
        assertEquals("lno1aaa+bbb", Bip21Parser.parse("bitcoin:?lno=lno1aaa%2Bbbb")!!.offer)
    }

    @Test
    fun `scheme match is ascii only`() {
        assertNull(Bip21Parser.parse("bıtcoin:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"))
        assertNotNull(Bip21Parser.parse("BITCOIN:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"))
    }
}
