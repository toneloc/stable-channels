package com.stablechannels.app.domain.send

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class Bech32Test {

    private fun failureOf(block: () -> Unit): Bech32.Failure? =
        try {
            block()
            null
        } catch (e: Bech32.DecodeException) {
            e.failure
        }

    /**
     * Kotlin has no checksum-only API, so a vector passes when decode succeeds or fails only at the
     * 5-to-8 bit conversion, which runs after the checksum check.
     */
    private fun assertChecksumValid(vector: String) {
        val failure = failureOf { Bech32.decode(vector) }
        assertTrue(
            "Checksum should verify for $vector, got $failure",
            failure == null || failure == Bech32.Failure.BITS_CONVERSION_FAILED,
        )
    }

    @Test
    fun `valid bech32 checksums verify`() {
        listOf(
                "A12UEL5L",
                "a12uel5l",
                "an83characterlonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1tt5tgs",
                "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw",
                "11qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc8247j",
                "split1checkupstagehandshakeupstreamerranterredcaperred2y9e3w",
                "?1ezyfcl",
            )
            .forEach(::assertChecksumValid)
    }

    @Test
    fun `valid bech32m checksums verify`() {
        listOf(
                "A1LQFN3A",
                "a1lqfn3a",
                "an83characterlonghumanreadablepartthatcontainsthetheexcludedcharactersbioandnumber11sg7hg6",
                "abcdef1l7aum6echk45nj3s0wdvt2fg8x9yrzpqzd3ryx",
                "11llllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllludsr8",
                "split1checkupstagehandshakeupstreamerranterredcaperredlc445v",
                "?1v759aa",
            )
            .forEach(::assertChecksumValid)
    }

    @Test
    fun `valid segwit addresses are accepted`() {
        listOf(
                "BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4" to "bc",
                "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7" to "tb",
                "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y" to
                    "bc",
                "BC1SW50QGDZ25J" to "bc",
                "bc1zw508d6qejxtdg4y5r3zarvaryvaxxpcs" to "bc",
                "tb1qqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesrxh6hy" to "tb",
                "tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c" to "tb",
                "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0" to "bc",
            )
            .forEach { (address, hrp) ->
                assertTrue("Expected valid: $address", Bech32.isValidSegwitAddress(address, hrp))
            }
    }

    @Test
    fun `invalid segwit addresses are rejected`() {
        listOf(
                "tc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq5zuyut" to "bc",
                "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqh2y7hd" to "bc",
                "tb1z0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqglt7rf" to "tb",
                "BC1S0XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQ54WELL" to "bc",
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh" to "bc",
                "tb1q0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq24jc47" to "tb",
                "bc1p38j9r5y49hruaue7wxjce0updqjuyyx0kh56v8s25huc6995vvpql3jow4" to "bc",
                "BC130XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQ7ZWS8R" to "bc",
                "bc1pw5dgrnzv" to "bc",
                "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7v8n0nx0muaewav253zgeav" to
                    "bc",
                "BC1QR508D6QEJXTDG4Y5R3ZARVARYV98GJ9P" to "bc",
                "tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq47Zagq" to "tb",
                "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7v07qwwzcrf" to "bc",
                "tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vpggkg4j" to "tb",
                "bc1gmk9yu" to "bc",
            )
            .forEach { (address, hrp) ->
                assertFalse("Expected invalid: $address", Bech32.isValidSegwitAddress(address, hrp))
            }
    }

    @Test
    fun `decodes lnurl with and without lightning scheme`() {
        val lnurl = "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
        val expected = "https://service.com/api/v1/lnurl/pay"
        assertEquals(expected, Bech32.decodeLnurl(lnurl).toString())
        assertEquals(expected, Bech32.decodeLnurl("lightning:$lnurl").toString())
        assertEquals(expected, Bech32.decodeLnurl("lightning://$lnurl").toString())
    }

    @Test
    fun `lnurl allows http for onion host and rejects http for clearnet`() {
        val tor = "lnurl1dp68gup69uhhxetjwe5kxefwdahxjmmw9acxz7gt0xmg5"
        assertEquals("http://service.onion/pay", Bech32.decodeLnurl(tor).toString())

        val clearnetHttp = "lnurl1dp68gup69uhhxetjwe5kxefwvdhk6tmsv9us85tvxr"
        assertEquals(
            Bech32.Failure.INSECURE_CLEARNET_SCHEME,
            failureOf { Bech32.decodeLnurl(clearnetHttp) },
        )
    }

    @Test
    fun `invalid bech32 strings are rejected`() {
        listOf(
                " 1nwldj5",
                "\u007F1axkwrx",
                "\u00801eym55h",
                "abc1\u007f23456",
                "an84characterslonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1569xm5",
                "pzry9x0s0muk",
                "1pzry9x0s0muk",
                "x1b4n0q5v",
                "li1dgmt3",
                "11llllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllludsr7",
            )
            .forEach { vector ->
                try {
                    Bech32.decode(vector)
                    fail("Should throw for invalid vector: $vector")
                } catch (_: Bech32.DecodeException) {}
            }
    }

    @Test
    fun `mixed case is rejected`() {
        assertEquals(Bech32.Failure.MIXED_CASE, failureOf { Bech32.decode("A12uel5l") })
    }

    @Test
    fun `oversized lnurl is rejected as invalid length`() {
        val oversized = "lnurl1" + "q".repeat(4097)
        assertEquals(Bech32.Failure.INVALID_LENGTH, failureOf { Bech32.decodeLnurl(oversized) })
    }

    @Test
    fun `lnurl with wrong hrp is rejected`() {
        assertEquals(
            Bech32.Failure.INVALID_HRP,
            failureOf { Bech32.decodeLnurl("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4") },
        )
    }

    @Test
    fun `segwit validation trims surrounding whitespace`() {
        assertTrue(Bech32.isValidSegwitAddress("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "bc"))
        assertTrue(
            Bech32.isValidSegwitAddress(
                "  \n\t bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 \r\n  ",
                "bc",
            )
        )
    }

    @Test
    fun `multibyte hrp character is rejected as invalid character`() {
        assertEquals(
            Bech32.Failure.INVALID_CHARACTER,
            failureOf { Bech32.decode("é1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4") },
        )
    }

    @Test
    fun `lnurl without url scheme is rejected as invalid url`() {
        assertEquals(
            Bech32.Failure.INVALID_URL,
            failureOf { Bech32.decodeLnurl("lnurl1wdjhyanfvdjjucm0d5hhqcte4z2zax") },
        )
    }

    @Test
    fun `lnurl with an underscore host keeps the host`() {
        val url = Bech32.decodeLnurl(encodeLnurl("https://my_host.example.com/p".toByteArray()))
        assertEquals("https://my_host.example.com/p", url.toString())
    }

    @Test
    fun `lnurl with invalid utf8 is rejected`() {
        val bytes = "https://example.com/".toByteArray() + byteArrayOf(0xC3.toByte(), 0x28)
        try {
            Bech32.decodeLnurl(encodeLnurl(bytes))
            fail("expected INVALID_UTF8")
        } catch (e: Bech32.DecodeException) {
            assertEquals(Bech32.Failure.INVALID_UTF8, e.failure)
        }
    }

    @Test
    fun `lightning scheme match is ascii only`() {
        assertEquals("lİghtning:abc", Bech32.stripLightningScheme("lİghtning:abc"))
        assertEquals("abc", Bech32.stripLightningScheme("LIGHTNING:abc"))
    }

    /** BIP-173 encoder for building LNURL fixtures; the codec under test only decodes. */
    private fun encodeLnurl(payload: ByteArray): String {
        val charset = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
        val data =
            Bech32.convertBits(
                IntArray(payload.size) { payload[it].toInt() and 0xFF },
                8,
                5,
                pad = true,
            )!!
        val hrp = "lnurl"
        val generators = intArrayOf(0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
        var chk = 1
        val values =
            hrp.map { it.code ushr 5 } +
                0 +
                hrp.map { it.code and 31 } +
                data.toList() +
                List(6) { 0 }
        for (v in values) {
            val top = chk ushr 25
            chk = ((chk and 0x1FFFFFF) shl 5) xor v
            for (i in 0 until 5) if ((top ushr i) and 1 != 0) chk = chk xor generators[i]
        }
        val polymod = chk xor 1
        val checksum = List(6) { (polymod ushr (5 * (5 - it))) and 31 }
        return hrp + "1" + (data.toList() + checksum).joinToString("") { charset[it].toString() }
    }
}
