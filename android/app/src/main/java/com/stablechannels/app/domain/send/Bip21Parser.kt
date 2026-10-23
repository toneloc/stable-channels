package com.stablechannels.app.domain.send

import java.math.BigDecimal
import java.net.URLDecoder

/** The parts of a BIP-21 `bitcoin:` URI that a send uses. */
data class Bip21Uri(
    val address: String,
    val amountSats: Long?,
    val lightning: String?,
    val offer: String?,
    val unhandledRequiredParameter: String?,
    val hasDuplicateParameter: Boolean,
)

/** Parses BIP-21 URIs, including the `lightning` and `lno` parameters of unified QR codes. */
object Bip21Parser {
    private val BTC_AMOUNT = Regex("^[0-9]+(?:\\.[0-9]{1,8})?$")
    private val MAX_SATS = BigDecimal(2_100_000_000_000_000L)
    private val SATS_PER_BTC = BigDecimal(100_000_000L)
    private val TRACKED_KEYS = setOf("amount", "lightning", "lno")

    /** Parses [uri]; returns null when it is not a `bitcoin:` URI. */
    fun parse(uri: String): Bip21Uri? {
        val body =
            when {
                uri.startsWithAscii("bitcoin://") -> uri.substring("bitcoin://".length)
                uri.startsWithAscii("bitcoin:") -> uri.substring("bitcoin:".length)
                else -> return null
            }
        val address = body.substringBefore('?').trim()
        val query = if ('?' in body) body.substringAfter('?') else ""

        var amountSats: Long? = null
        var lightning: String? = null
        var offer: String? = null
        var unhandledRequired: String? = null
        var duplicate = false
        val seen = mutableSetOf<String>()

        for (pair in query.split('&').filter { it.isNotEmpty() }) {
            val name = decode(pair.substringBefore('=')).lowercase()
            val value = if ('=' in pair) decode(pair.substringAfter('=')) else null
            if (name.startsWith("req-") || name == "pop") unhandledRequired = name
            if (name in TRACKED_KEYS && !seen.add(name)) duplicate = true
            when {
                name == "amount" && value != null -> amountSats = btcToSats(value)
                name == "lightning" && !value.isNullOrEmpty() -> lightning = value
                name == "lno" && !value.isNullOrEmpty() -> offer = value
            }
        }
        return Bip21Uri(address, amountSats, lightning, offer, unhandledRequired, duplicate)
    }

    /**
     * Converts a BIP-21 BTC amount to sats. Rejects signs, exponents, more than 8 decimals, zero
     * and anything above 21 million BTC.
     */
    fun btcToSats(btc: String): Long? {
        if (!BTC_AMOUNT.matches(btc)) return null
        val sats = BigDecimal(btc).multiply(SATS_PER_BTC)
        if (sats.signum() <= 0 || sats > MAX_SATS) return null
        return sats.longValueExact()
    }

    private fun decode(text: String): String = runCatching {
        // A literal "+" is kept: BOLT12 offers use it, and BIP-21 values are only percent-encoded.
        URLDecoder.decode(text.replace("+", "%2B"), Charsets.UTF_8.name())
    }
        .getOrDefault(text)
}
