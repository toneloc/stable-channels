package com.stablechannels.app.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class QRCodeUtilsTest {

    // -----------------------------------------------------------------------
    // normalizeAddress
    // -----------------------------------------------------------------------

    @Test
    fun `lowercase bech32 mainnet passes through unchanged`() {
        val addr = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        assertEquals(addr, QRCodeUtils.normalizeAddress(addr))
    }

    @Test
    fun `lowercase bech32 testnet passes through unchanged`() {
        val addr = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        assertEquals(addr, QRCodeUtils.normalizeAddress(addr))
    }

    @Test
    fun `lowercase bech32 regtest passes through unchanged`() {
        val addr = "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"
        assertEquals(addr, QRCodeUtils.normalizeAddress(addr))
    }

    @Test
    fun `uppercase bech32 mainnet is lowercased`() {
        assertEquals(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            QRCodeUtils.normalizeAddress("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4")
        )
    }

    @Test
    fun `uppercase bech32 testnet is lowercased`() {
        assertEquals(
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            QRCodeUtils.normalizeAddress("TB1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KXPJZSX")
        )
    }

    @Test
    fun `uppercase bech32 regtest is lowercased`() {
        assertEquals(
            "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080",
            QRCodeUtils.normalizeAddress("BCRT1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KYGT080")
        )
    }

    @Test
    fun `mixed-case bech32 is lowercased`() {
        assertEquals(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            QRCodeUtils.normalizeAddress("Bc1Qw508d6qejXTDG4Y5R3ZARVARY0C5XW7KV8F3T4")
        )
    }

    @Test
    fun `base58 mainnet address case is preserved`() {
        val addr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        assertEquals(addr, QRCodeUtils.normalizeAddress(addr))
    }

    @Test
    fun `base58 p2sh address case is preserved`() {
        val addr = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
        assertEquals(addr, QRCodeUtils.normalizeAddress(addr))
    }

    @Test
    fun `base58 testnet address case is preserved`() {
        val addr = "2N3oefVeg6stiTb5Kh3ozCRPgMBLCnBKE1m"
        assertEquals(addr, QRCodeUtils.normalizeAddress(addr))
    }

    @Test
    fun `empty string returns empty`() {
        assertEquals("", QRCodeUtils.normalizeAddress(""))
    }

    @Test
    fun `whitespace-only string returns empty`() {
        assertEquals("", QRCodeUtils.normalizeAddress("   "))
    }

    @Test
    fun `whitespace around address is trimmed`() {
        assertEquals(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            QRCodeUtils.normalizeAddress("  bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4  ")
        )
    }

    @Test
    fun `non-address string with uppercase is preserved`() {
        // Not a bech32 prefix, not a base58 prefix -- uppercase preserved
        val input = "SomeRandomString"
        assertEquals(input, QRCodeUtils.normalizeAddress(input))
    }

    @Test
    fun `short string shorter than prefix is safe`() {
        assertEquals("bc", QRCodeUtils.normalizeAddress("bc"))
        assertEquals("b", QRCodeUtils.normalizeAddress("b"))
        assertEquals("1", QRCodeUtils.normalizeAddress("1"))
    }

    // -----------------------------------------------------------------------
    // stripUriPrefix
    // -----------------------------------------------------------------------

    @Test
    fun `strips bitcoin colon prefix`() {
        assertEquals(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            QRCodeUtils.stripUriPrefix("bitcoin:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")
        )
    }

    @Test
    fun `strips BITCOIN colon prefix case-insensitively`() {
        assertEquals(
            "bc1qtest",
            QRCodeUtils.stripUriPrefix("BITCOIN:BC1QTEST")
        )
    }

    @Test
    fun `strips bitcoin double-slash prefix`() {
        assertEquals(
            "bc1qtest",
            QRCodeUtils.stripUriPrefix("bitcoin://BC1QTEST")
        )
    }

    @Test
    fun `strips lightning colon prefix`() {
        assertEquals(
            "lnbc1pvjluezpp5",
            QRCodeUtils.stripUriPrefix("lightning:lnbc1pvjluezpp5")
        )
    }

    @Test
    fun `strips lightning double-slash prefix`() {
        assertEquals(
            "lnbc1pvjluezpp5",
            QRCodeUtils.stripUriPrefix("lightning://lnbc1pvjluezpp5")
        )
    }

    @Test
    fun `strips query parameters after question mark`() {
        assertEquals(
            "bc1qtest",
            QRCodeUtils.stripUriPrefix("bitcoin:BC1QTEST?amount=0.001&label=test")
        )
    }

    @Test
    fun `handles NBSP in input`() {
        val input = "bitcoin:\u00A0bc1qtest"
        // NBSP is replaced with space, then trimmed after prefix strip
        val result = QRCodeUtils.stripUriPrefix(input)
        assertEquals("bc1qtest", result)
    }

    @Test
    fun `takes first non-blank line from multiline input`() {
        val input = "\n\nbitcoin:BC1QTEST\nsecond line\n"
        assertEquals("bc1qtest", QRCodeUtils.stripUriPrefix(input))
    }

    @Test
    fun `empty input returns empty`() {
        assertEquals("", QRCodeUtils.stripUriPrefix(""))
    }

    @Test
    fun `whitespace-only input returns empty`() {
        assertEquals("", QRCodeUtils.stripUriPrefix("   \n  \n  "))
    }

    @Test
    fun `plain address without prefix is normalized`() {
        assertEquals(
            "bc1qtest",
            QRCodeUtils.stripUriPrefix("BC1QTEST")
        )
    }

    @Test
    fun `base58 address without prefix preserves case`() {
        val addr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        assertEquals(addr, QRCodeUtils.stripUriPrefix(addr))
    }

    // -----------------------------------------------------------------------
    // generateBitcoinUri
    // -----------------------------------------------------------------------

    @Test
    fun `generates uri without amount`() {
        assertEquals(
            "bitcoin:bc1qtest",
            QRCodeUtils.generateBitcoinUri("bc1qtest")
        )
    }

    @Test
    fun `generates uri with amount`() {
        assertEquals(
            "bitcoin:bc1qtest?amount=0.001",
            QRCodeUtils.generateBitcoinUri("bc1qtest", "0.001")
        )
    }

    @Test
    fun `normalizes uppercase address in uri`() {
        assertEquals(
            "bitcoin:bc1qtest",
            QRCodeUtils.generateBitcoinUri("BC1QTEST")
        )
    }

    @Test
    fun `blank amount treated as no amount`() {
        assertEquals(
            "bitcoin:bc1qtest",
            QRCodeUtils.generateBitcoinUri("bc1qtest", "")
        )
    }

    @Test
    fun `null amount treated as no amount`() {
        assertEquals(
            "bitcoin:bc1qtest",
            QRCodeUtils.generateBitcoinUri("bc1qtest", null)
        )
    }

    @Test
    fun `preserves base58 case in uri`() {
        assertEquals(
            "bitcoin:1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
            QRCodeUtils.generateBitcoinUri("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa")
        )
    }

    // -----------------------------------------------------------------------
    // isValidPaymentString
    // -----------------------------------------------------------------------

    @Test
    fun `recognizes bech32 mainnet address`() {
        assertTrue(QRCodeUtils.isValidPaymentString("bc1qtest"))
    }

    @Test
    fun `recognizes uppercase bech32`() {
        assertTrue(QRCodeUtils.isValidPaymentString("BC1QTEST"))
    }

    @Test
    fun `recognizes bech32 testnet address`() {
        assertTrue(QRCodeUtils.isValidPaymentString("tb1qtest"))
    }

    @Test
    fun `recognizes bech32 regtest address`() {
        assertTrue(QRCodeUtils.isValidPaymentString("bcrt1qtest"))
    }

    @Test
    fun `recognizes base58 mainnet prefix 1`() {
        assertTrue(QRCodeUtils.isValidPaymentString("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"))
    }

    @Test
    fun `recognizes base58 p2sh prefix 3`() {
        assertTrue(QRCodeUtils.isValidPaymentString("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"))
    }

    @Test
    fun `recognizes base58 testnet prefix 2`() {
        assertTrue(QRCodeUtils.isValidPaymentString("2N3oefVeg6stiTb5Kh3ozCRPgMBLCnBKE1m"))
    }

    @Test
    fun `recognizes base58 testnet prefix m`() {
        assertTrue(QRCodeUtils.isValidPaymentString("mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn"))
    }

    @Test
    fun `recognizes base58 testnet prefix n`() {
        assertTrue(QRCodeUtils.isValidPaymentString("n1ww1VkBbNk2KeEDqN3nR8EVJi5tKBczAo"))
    }

    @Test
    fun `recognizes bolt11 invoice lnbc`() {
        assertTrue(QRCodeUtils.isValidPaymentString("lnbc1pvjluezpp5"))
    }

    @Test
    fun `recognizes bolt11 invoice lntb`() {
        assertTrue(QRCodeUtils.isValidPaymentString("lntb1pvjluezpp5"))
    }

    @Test
    fun `recognizes bolt12 offer lno`() {
        assertTrue(QRCodeUtils.isValidPaymentString("lno1qgsqvgjwp"))
    }

    @Test
    fun `recognizes regtest bolt11 lnbcrt`() {
        assertTrue(QRCodeUtils.isValidPaymentString("lnbcrt1pvjluez"))
    }

    @Test
    fun `rejects empty string`() {
        assertFalse(QRCodeUtils.isValidPaymentString(""))
    }

    @Test
    fun `rejects whitespace-only string`() {
        assertFalse(QRCodeUtils.isValidPaymentString("   "))
    }

    @Test
    fun `rejects unrecognized prefix`() {
        assertFalse(QRCodeUtils.isValidPaymentString("xpub6CUGRUon"))
    }

    @Test
    fun `handles leading whitespace before valid prefix`() {
        assertTrue(QRCodeUtils.isValidPaymentString("  bc1qtest"))
    }
}
