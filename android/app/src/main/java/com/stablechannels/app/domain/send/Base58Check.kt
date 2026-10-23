package com.stablechannels.app.domain.send

import java.security.MessageDigest

/** Base58Check decoder and address checksum verifier. */
object Base58Check {
    private const val ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

    /** P2PKH and P2SH version bytes for mainnet (0x00, 0x05) and test networks (0x6F, 0xC4). */
    private val ADDRESS_VERSIONS = setOf(0x00, 0x05, 0x6F, 0xC4)

    /** Decodes [input] to bytes, or null when it contains a non-Base58 character. */
    fun decode(input: String): ByteArray? {
        val trimmed = input.trim()
        if (trimmed.isEmpty()) return null

        val zeroes = trimmed.takeWhile { it == '1' }.length
        val b256 = IntArray(trimmed.length * 733 / 1000 + 1)
        for (char in trimmed) {
            var carry = ALPHABET.indexOf(char)
            if (carry < 0) return null
            for (j in b256.indices.reversed()) {
                carry += 58 * b256[j]
                b256[j] = carry and 0xFF
                carry = carry ushr 8
            }
            if (carry != 0) return null
        }

        val start = b256.indexOfFirst { it != 0 }.let { if (it < 0) b256.size else it }
        return ByteArray(zeroes) + ByteArray(b256.size - start) { b256[start + it].toByte() }
    }

    /**
     * True when [address] is a 25-byte P2PKH or P2SH payload with a valid double-SHA256 checksum.
     */
    fun isValidAddress(address: String): Boolean {
        val decoded = decode(address) ?: return false
        if (decoded.size != 25) return false
        val payload = decoded.copyOfRange(0, 21)
        val sha = MessageDigest.getInstance("SHA-256")
        val expected = sha.digest(sha.digest(payload)).copyOfRange(0, 4)
        if (!expected.contentEquals(decoded.copyOfRange(21, 25))) return false
        return (payload[0].toInt() and 0xFF) in ADDRESS_VERSIONS
    }
}
