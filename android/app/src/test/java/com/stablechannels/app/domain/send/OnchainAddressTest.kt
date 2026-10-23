package com.stablechannels.app.domain.send

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OnchainAddressTest {
    private val mainnetSegwit = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
    private val mainnetTaproot = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"
    private val mainnetP2pkh = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
    private val mainnetP2sh = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
    private val testSegwit = "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7"
    private val testP2pkhM = "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn"
    private val testP2pkhN = "n4rZHAPGXCu8bYchjzJhK3V7VVreascJxe"
    private val testP2sh = "2MsFDzHRUAMpjHxKyoEHU3aMCMsVtMqs1PV"
    private val regtestSegwit = "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"

    private val mainnet = listOf(mainnetSegwit, mainnetTaproot, mainnetP2pkh, mainnetP2sh)
    private val testShared = listOf(testSegwit, testP2pkhM, testP2pkhN, testP2sh)

    @Test
    fun `mainnet accepts mainnet addresses only`() {
        mainnet.forEach { assertTrue(it, OnchainAddress.isValid(it, BitcoinNetwork.BITCOIN)) }
        (testShared + regtestSegwit).forEach {
            assertFalse(it, OnchainAddress.isValid(it, BitcoinNetwork.BITCOIN))
        }
    }

    @Test
    fun `testnet accepts tb1 m n and 2 prefixes`() {
        testShared.forEach { assertTrue(it, OnchainAddress.isValid(it, BitcoinNetwork.TESTNET)) }
        (mainnet + regtestSegwit).forEach {
            assertFalse(it, OnchainAddress.isValid(it, BitcoinNetwork.TESTNET))
        }
    }

    @Test
    fun `signet shares testnet prefixes`() {
        testShared.forEach { assertTrue(it, OnchainAddress.isValid(it, BitcoinNetwork.SIGNET)) }
        (mainnet + regtestSegwit).forEach {
            assertFalse(it, OnchainAddress.isValid(it, BitcoinNetwork.SIGNET))
        }
    }

    @Test
    fun `regtest accepts bcrt1 and test base58 but not tb1 or mainnet`() {
        assertTrue(OnchainAddress.isValid(regtestSegwit, BitcoinNetwork.REGTEST))
        listOf(testP2pkhM, testP2pkhN, testP2sh).forEach {
            assertTrue(it, OnchainAddress.isValid(it, BitcoinNetwork.REGTEST))
        }
        (mainnet + testSegwit).forEach {
            assertFalse(it, OnchainAddress.isValid(it, BitcoinNetwork.REGTEST))
        }
    }

    @Test
    fun `without network any valid address is accepted`() {
        (mainnet + testShared + regtestSegwit).forEach {
            assertTrue(it, OnchainAddress.isValid(it))
        }
    }

    @Test
    fun `invalid addresses are rejected`() {
        assertFalse(OnchainAddress.isValid("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"))
        assertFalse(OnchainAddress.isValid("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh"))
        assertFalse(OnchainAddress.isValid("short"))
        assertFalse(OnchainAddress.isValid("xc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"))
    }

    @Test
    fun `uppercase segwit is valid`() {
        assertTrue(OnchainAddress.isValid(mainnetSegwit.uppercase(), BitcoinNetwork.BITCOIN))
    }

    @Test
    fun `normalize lowercases segwit and trims`() {
        assertEquals(mainnetSegwit, OnchainAddress.normalize("  ${mainnetSegwit.uppercase()} \n"))
        assertEquals(testSegwit, OnchainAddress.normalize(testSegwit.uppercase()))
        assertEquals(regtestSegwit, OnchainAddress.normalize(regtestSegwit.uppercase()))
    }

    @Test
    fun `normalize keeps base58 case`() {
        assertEquals(mainnetP2pkh, OnchainAddress.normalize(" $mainnetP2pkh "))
        assertEquals(testP2sh, OnchainAddress.normalize(testP2sh))
    }
}
