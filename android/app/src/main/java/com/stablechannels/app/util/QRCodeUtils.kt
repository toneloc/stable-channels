package com.stablechannels.app.util

/**
 * Utility functions for processing QR code payloads containing
 * Lightning invoices, Bolt12 offers, and Bitcoin addresses.
 */
object QRCodeUtils {

    private val URI_PREFIXES = listOf("bitcoin:", "lightning:", "BITCOIN:", "LIGHTNING:")

    /**
     * Strips URI scheme prefixes (bitcoin:, lightning:, BITCOIN:, LIGHTNING:)
     * and query parameters (everything after '?') from a raw QR code string.
     *
     * @param raw The raw decoded QR code string
     * @return The cleaned payment payload
     */
    fun stripUriPrefix(raw: String): String {
        var result = raw.trim()

        // Strip known URI prefixes
        for (prefix in URI_PREFIXES) {
            if (result.startsWith(prefix)) {
                result = result.removePrefix(prefix)
                break
            }
        }

        // Strip query parameters (everything after '?')
        val queryIndex = result.indexOf('?')
        if (queryIndex >= 0) {
            result = result.substring(0, queryIndex)
        }

        return result
    }

    /**
     * Checks whether a string looks like a valid payment string based on known prefixes.
     * Recognizes: Bolt11 (lnbc, lntb, lnts), Bolt12 (lno), on-chain (bc1, 1, 3, tb1).
     *
     * @param value The string to validate
     * @return true if the string starts with a recognized payment prefix
     */
    fun isValidPaymentString(value: String): Boolean {
        val lower = value.trim().lowercase()
        return lower.startsWith("lnbc") ||
                lower.startsWith("lntb") ||
                lower.startsWith("lnts") ||
                lower.startsWith("lno") ||
                lower.startsWith("bc1") ||
                lower.startsWith("tb1") ||
                value.trim().startsWith("1") ||
                value.trim().startsWith("3")
    }
}
