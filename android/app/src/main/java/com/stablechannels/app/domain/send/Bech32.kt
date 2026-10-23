package com.stablechannels.app.domain.send

import java.net.URI
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** BIP-173 Bech32 and BIP-350 Bech32m decoder. */
object Bech32 {
    enum class Failure {
        INVALID_LENGTH,
        INVALID_CHARACTER,
        MIXED_CASE,
        MISSING_HRP,
        INVALID_HRP,
        INVALID_CHECKSUM,
        BITS_CONVERSION_FAILED,
        INVALID_UTF8,
        INVALID_URL,
        INSECURE_CLEARNET_SCHEME,
    }

    class DecodeException(val failure: Failure) : Exception(failure.name)

    enum class ChecksumType {
        BECH32,
        BECH32M,
    }

    class Decoded(val hrp: String, val data: ByteArray)

    private const val CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    private const val BECH32_CONST = 1
    private const val BECH32M_CONST = 0x2BC830A3
    private const val MAX_ADDRESS_LENGTH = 90
    private const val MAX_LNURL_LENGTH = 4096
    private val GENERATORS = intArrayOf(0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)

    private class Parsed(val hrp: String, val payload5Bit: IntArray, val checksumType: ChecksumType)

    private fun polymod(hrp: String, data: IntArray): Int {
        var chk = 1
        fun step(value: Int) {
            val top = chk ushr 25
            chk = ((chk and 0x1FFFFFF) shl 5) xor value
            for (i in 0 until 5) if ((top ushr i) and 1 != 0) chk = chk xor GENERATORS[i]
        }
        hrp.forEach { step(it.code ushr 5) }
        step(0)
        hrp.forEach { step(it.code and 31) }
        data.forEach { step(it) }
        return chk
    }

    private fun parse(input: String, limitLength: Boolean): Parsed {
        if (limitLength && input.length > MAX_ADDRESS_LENGTH)
            throw DecodeException(Failure.INVALID_LENGTH)
        if (input.length < 8) throw DecodeException(Failure.INVALID_LENGTH)
        if (input.any { it.isLowerCase() } && input.any { it.isUpperCase() }) {
            throw DecodeException(Failure.MIXED_CASE)
        }

        val lower = input.lowercase()
        val separator = lower.lastIndexOf('1')
        if (separator < 1) throw DecodeException(Failure.MISSING_HRP)
        val hrp = lower.substring(0, separator)
        if (hrp.any { it.code !in 33..126 }) throw DecodeException(Failure.INVALID_CHARACTER)

        val dataPart = lower.substring(separator + 1)
        if (dataPart.length < 6) throw DecodeException(Failure.INVALID_LENGTH)
        val values =
            IntArray(dataPart.length) { i ->
                CHARSET.indexOf(dataPart[i]).also {
                    if (it < 0) throw DecodeException(Failure.INVALID_CHARACTER)
                }
            }

        val checksumType =
            when (polymod(hrp, values)) {
                BECH32_CONST -> ChecksumType.BECH32
                BECH32M_CONST -> ChecksumType.BECH32M
                else -> throw DecodeException(Failure.INVALID_CHECKSUM)
            }
        return Parsed(hrp, values.copyOf(values.size - 6), checksumType)
    }

    /**
     * Regroups [data] from [fromBits]-bit to [toBits]-bit values; null when the padding is invalid.
     */
    fun convertBits(data: IntArray, fromBits: Int, toBits: Int, pad: Boolean): IntArray? {
        var acc = 0
        var bits = 0
        val out = ArrayList<Int>(data.size * fromBits / toBits + 1)
        val maxValue = (1 shl toBits) - 1
        val maxAcc = (1 shl (fromBits + toBits - 1)) - 1
        for (value in data) {
            if (value ushr fromBits != 0) return null
            acc = ((acc shl fromBits) or value) and maxAcc
            bits += fromBits
            while (bits >= toBits) {
                bits -= toBits
                out.add((acc ushr bits) and maxValue)
            }
        }
        if (pad) {
            if (bits > 0) out.add((acc shl (toBits - bits)) and maxValue)
        } else if (bits >= fromBits || ((acc shl (toBits - bits)) and maxValue) != 0) {
            return null
        }
        return out.toIntArray()
    }

    /** Decodes a Bech32 or Bech32m string into its lowercased HRP and 8-bit payload. */
    fun decode(input: String, limitLength: Boolean = true): Decoded {
        val parsed = parse(input, limitLength)
        val bytes =
            convertBits(parsed.payload5Bit, 5, 8, pad = false)
                ?: throw DecodeException(Failure.BITS_CONVERSION_FAILED)
        return Decoded(parsed.hrp, ByteArray(bytes.size) { bytes[it].toByte() })
    }

    /**
     * True when [address] is a native segwit address for [expectedHrp]: version 0 with Bech32 and a
     * 20 or 32 byte program (BIP-173), or version 1 to 16 with Bech32m and 2 to 40 bytes (BIP-350).
     */
    fun isValidSegwitAddress(address: String, expectedHrp: String): Boolean {
        val parsed =
            runCatching { parse(address.trim(), limitLength = true) }.getOrNull() ?: return false
        if (parsed.hrp != expectedHrp.lowercase() || parsed.payload5Bit.isEmpty()) return false
        val version = parsed.payload5Bit[0]
        if (version > 16) return false
        val program =
            convertBits(
                parsed.payload5Bit.copyOfRange(1, parsed.payload5Bit.size),
                5,
                8,
                pad = false,
            ) ?: return false
        return if (version == 0) {
            parsed.checksumType == ChecksumType.BECH32 && (program.size == 20 || program.size == 32)
        } else {
            parsed.checksumType == ChecksumType.BECH32M && program.size in 2..40
        }
    }

    /**
     * Decodes an LNURL (LUD-01, `lnurl1...`, optionally prefixed with `lightning:`) into its URL.
     * Clearnet hosts must use HTTPS; `.onion` hosts may use HTTP.
     */
    fun decodeLnurl(input: String): URI {
        val clean = stripLightningScheme(input.trim()).trim()
        if (clean.length > MAX_LNURL_LENGTH) throw DecodeException(Failure.INVALID_LENGTH)

        val parsed = parse(clean, limitLength = false)
        if (parsed.hrp != "lnurl") throw DecodeException(Failure.INVALID_HRP)
        if (parsed.checksumType != ChecksumType.BECH32)
            throw DecodeException(Failure.INVALID_CHECKSUM)
        val bytes =
            convertBits(parsed.payload5Bit, 5, 8, pad = false)
                ?: throw DecodeException(Failure.BITS_CONVERSION_FAILED)

        val text =
            try {
                Charsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(ByteArray(bytes.size) { bytes[it].toByte() }))
                    .toString()
            } catch (_: CharacterCodingException) {
                throw DecodeException(Failure.INVALID_UTF8)
            }
        val url =
            runCatching { URI(text) }.getOrNull() ?: throw DecodeException(Failure.INVALID_URL)
        val scheme = url.scheme?.lowercase() ?: throw DecodeException(Failure.INVALID_URL)
        // URI leaves host null for names it treats as registry-based (e.g. with "_"); use the
        // authority.
        val host =
            (url.host ?: url.rawAuthority?.substringAfterLast('@')?.substringBefore(':'))
                ?.lowercase()
                ?.takeIf { it.isNotEmpty() } ?: throw DecodeException(Failure.INVALID_URL)

        val allowed =
            if (host.endsWith(".onion")) scheme == "http" || scheme == "https"
            else scheme == "https"
        if (!allowed) throw DecodeException(Failure.INSECURE_CLEARNET_SCHEME)
        return url
    }

    /** Removes a leading `lightning://` or `lightning:` scheme, case-insensitively. */
    internal fun stripLightningScheme(input: String): String =
        when {
            input.startsWithAscii("lightning://") -> input.substring("lightning://".length)
            input.startsWithAscii("lightning:") -> input.substring("lightning:".length)
            else -> input
        }
}

/**
 * Case-insensitive prefix match over ASCII only, so letters such as the dotless "ı" never match "i"
 * the way Unicode case folding would.
 */
internal fun String.startsWithAscii(prefix: String): Boolean =
    length >= prefix.length &&
        (0 until prefix.length).all { this[it].code < 128 } &&
        startsWith(prefix, ignoreCase = true)
