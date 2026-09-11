package com.stablechannels.app.util

/**
 * Utility functions for processing QR code payloads containing
 * Lightning invoices, Bolt12 offers, and Bitcoin addresses.
 */
object QRCodeUtils {

    private const val BITCOIN_SCHEME = "bitcoin:"
    private const val LIGHTNING_SCHEME = "lightning:"

    /**
     * Strips URI scheme prefixes (bitcoin:, lightning:, BITCOIN:, LIGHTNING:)
     * and query parameters (everything after '?') from a raw QR code string.
     *
     * @param raw The raw decoded QR code string
     * @return The cleaned payment payload
     */
    fun stripUriPrefix(raw: String): String {
        var s = raw.lines().firstOrNull { it.isNotBlank() }?.replace("\u00A0", " ")?.trim() ?: ""
        if (s.isEmpty()) return ""

        if (s.startsWith("bitcoin://", ignoreCase = true)) {
            s = s.substring(10)
        } else if (s.startsWith(BITCOIN_SCHEME, ignoreCase = true)) {
            s = s.substring(BITCOIN_SCHEME.length)
        } else if (s.startsWith("lightning://", ignoreCase = true)) {
            s = s.substring(12)
        } else if (s.startsWith(LIGHTNING_SCHEME, ignoreCase = true)) {
            s = s.substring(LIGHTNING_SCHEME.length)
        }

        val queryIndex = s.indexOf('?')
        if (queryIndex >= 0) {
            s = s.substring(0, queryIndex)
        }

        return normalizeAddress(s)
    }

    /**
     * Normalizes a Bitcoin address according to BIP-173 / BIP-350 specifications.
     * Native SegWit and Taproot (bc1, tb1, bcrt1) addresses are converted to lowercase.
     * Base58 addresses (1, 3, 2, m, n) retain their exact case.
     */
    fun normalizeAddress(raw: String): String {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) return trimmed

        var hasUpper = false
        for (i in 0 until trimmed.length) {
            val c = trimmed[i]
            if (c in 'A'..'Z') {
                hasUpper = true
                break
            }
        }
        if (!hasUpper) return trimmed

        val isBech32 = trimmed.startsWith("bc1", ignoreCase = true) ||
            trimmed.startsWith("tb1", ignoreCase = true) ||
            trimmed.startsWith("bcrt1", ignoreCase = true)

        return if (isBech32) {
            trimmed.lowercase()
        } else {
            trimmed
        }
    }

    /**
     * Generates a BIP-21 URI with normalized address and optional amount.
     */
    fun generateBitcoinUri(address: String, amount: String? = null): String {
        val normalized = normalizeAddress(address)
        return if (amount.isNullOrBlank()) {
            "bitcoin:$normalized"
        } else {
            "bitcoin:$normalized?amount=$amount"
        }
    }

    /**
     * Checks whether a string looks like a valid payment string based on known prefixes.
     * Recognizes: Bolt11 (lnbc, lntb, lnts), Bolt12 (lno), on-chain (bc1, 1, 3, tb1).
     *
     * @param value The string to validate
     * @return true if the string starts with a recognized payment prefix
     */
    fun isValidPaymentString(value: String): Boolean {
        val trimmed = value.trim()
        if (trimmed.isEmpty()) return false
        val firstChar = trimmed[0]
        if (firstChar == '1' || firstChar == '3' || firstChar == '2' || firstChar == 'm' || firstChar == 'n') return true

        return trimmed.startsWith("bc1", ignoreCase = true) ||
            trimmed.startsWith("tb1", ignoreCase = true) ||
            trimmed.startsWith("bcrt1", ignoreCase = true) ||
            trimmed.startsWith("lnbc", ignoreCase = true) ||
            trimmed.startsWith("lntb", ignoreCase = true) ||
            trimmed.startsWith("lnts", ignoreCase = true) ||
            trimmed.startsWith("lnbcrt", ignoreCase = true) ||
            trimmed.startsWith("lno", ignoreCase = true)
    }
}
