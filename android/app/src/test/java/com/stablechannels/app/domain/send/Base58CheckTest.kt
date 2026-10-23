package com.stablechannels.app.domain.send

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class Base58CheckTest {

    @Test
    fun `valid mainnet p2pkh`() {
        assertTrue(Base58Check.isValidAddress("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"))
    }

    @Test
    fun `valid mainnet p2sh`() {
        assertTrue(Base58Check.isValidAddress("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"))
    }

    @Test
    fun `valid testnet p2pkh`() {
        assertTrue(Base58Check.isValidAddress("mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn"))
    }

    @Test
    fun `valid testnet p2sh`() {
        assertTrue(Base58Check.isValidAddress("2MsFDzHRUAMpjHxKyoEHU3aMCMsVtMqs1PV"))
    }

    @Test
    fun `invalid checksum is rejected`() {
        assertFalse(Base58Check.isValidAddress("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"))
    }

    @Test
    fun `characters outside the alphabet are rejected`() {
        listOf("0", "O", "I", "l").forEach { c ->
            assertFalse(c, Base58Check.isValidAddress("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfN$c"))
        }
    }

    @Test
    fun `empty and whitespace input is rejected`() {
        assertFalse(Base58Check.isValidAddress(""))
        assertFalse(Base58Check.isValidAddress("   \n\t "))
    }

    @Test
    fun `truncated address is rejected`() {
        assertFalse(Base58Check.isValidAddress("1A1zP1eP5QGefi2"))
    }

    @Test
    fun `unsupported version byte is rejected`() {
        assertFalse(Base58Check.isValidAddress("D5ERdEN1gsouFSs7zsq7VYJxyWP6dP28H1"))
    }

    @Test
    fun `leading ones decode to zero bytes`() {
        assertArrayEquals(byteArrayOf(0, 0), Base58Check.decode("11"))

        val address = "1111111111111111111114oLvT2"
        val decoded = Base58Check.decode(address)
        assertNotNull(decoded)
        assertEquals(25, decoded!!.size)
        assertTrue(decoded.copyOfRange(0, 21).all { it == 0.toByte() })
        assertTrue(Base58Check.isValidAddress(address))
    }

    @Test
    fun `decode of empty input returns null`() {
        assertNull(Base58Check.decode(""))
        assertNull(Base58Check.decode("   "))
    }
}
