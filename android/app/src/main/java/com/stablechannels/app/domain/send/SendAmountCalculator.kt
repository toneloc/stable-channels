package com.stablechannels.app.domain.send

import java.math.BigDecimal
import java.math.RoundingMode

/** Unit the user types an amount in. Labels and symbols live in the UI. */
enum class SendAmountUnit(val maxDecimals: Int) {
    USD(2),
    SATS(0),
    BTC(8),
}

/**
 * Converts typed amounts to and from sats.
 *
 * Every conversion goes through [BigDecimal], never `Double`: a binary double cannot represent most
 * decimal inputs exactly ("0.0003" * 1e8 is 29999.999999999996), so flooring a double product drops
 * a sat on ordinary amounts. Every conversion rounds down to the unit's precision, so a shown or
 * confirmed amount is never larger than what the user typed or what is sent.
 */
object SendAmountCalculator {
    private val SATS_PER_BTC = BigDecimal(100_000_000L)
    private const val SATS_PER_BTC_LONG = 100_000_000L

    /** 21,000,000 BTC in sats; anything above is not a real amount. */
    private val MAX_SATS = BigDecimal(2_100_000_000_000_000L)
    private val HUNDRED = BigDecimal(100)

    /**
     * Parses user text as a non-negative decimal. Accepts digits with at most one point (".5" and
     * "5." included); rejects signs, exponents, separators and anything else.
     */
    fun parseDecimal(text: String): BigDecimal? {
        var trimmed = text.trim()
        if (trimmed.isEmpty()) return null
        if (!trimmed.all { it in '0'..'9' || it == '.' } || trimmed.count { it == '.' } > 1)
            return null
        if (trimmed.startsWith(".")) trimmed = "0$trimmed"
        trimmed = trimmed.removeSuffix(".")
        return trimmed.takeIf { it.isNotEmpty() }?.let(::BigDecimal)
    }

    /** Whole sats for a non-negative amount, or null when it is below 1 or above 21M BTC. */
    private fun satsValue(value: BigDecimal): Long? {
        val whole = value.setScale(0, RoundingMode.DOWN)
        if (whole < BigDecimal.ONE || whole > MAX_SATS) return null
        return whole.longValueExact()
    }

    fun satsFromBtc(btc: BigDecimal): Long? = satsValue(btc.multiply(SATS_PER_BTC))

    fun satsFromUsd(usd: BigDecimal, btcPrice: Double): Long? {
        if (!btcPrice.isFinite() || btcPrice <= 0) return null
        // Multiply before dividing so the single division floors the exact quotient.
        return satsValue(
            usd.multiply(SATS_PER_BTC).divide(BigDecimal.valueOf(btcPrice), 0, RoundingMode.DOWN)
        )
    }

    /** USD cents for [sats] at [btcPrice], floored to a whole cent. */
    fun centsFromSats(sats: Long, btcPrice: Double): Long? {
        if (!btcPrice.isFinite() || btcPrice <= 0 || sats < 0) return null
        val cents =
            BigDecimal(sats)
                .multiply(BigDecimal.valueOf(btcPrice))
                .multiply(HUNDRED)
                .divide(SATS_PER_BTC, 0, RoundingMode.DOWN)
        return cents.takeIf { it <= MAX_SATS }?.longValueExact()
    }

    /** Exact BTC string for [sats]: "0.00030000", or "0.0003" with [trimTrailingZeros]. */
    fun btcString(sats: Long, trimTrailingZeros: Boolean): String {
        val text =
            "${sats / SATS_PER_BTC_LONG}." + (sats % SATS_PER_BTC_LONG).toString().padStart(8, '0')
        return if (trimTrailingZeros) text.trimEnd('0').removeSuffix(".") else text
    }

    /** Exact USD string for a cent amount: "12.34". */
    fun usdString(cents: Long): String =
        "${cents / 100}." + (cents % 100).toString().padStart(2, '0')

    /**
     * Cleans typed text for [unit]. Extra precision is truncated, so this can only lower an amount.
     */
    fun normalizeInput(text: String, unit: SendAmountUnit): String {
        val value = parseDecimal(text) ?: return ""
        val scaled =
            when (unit) {
                SendAmountUnit.USD -> value.multiply(HUNDRED)
                SendAmountUnit.SATS -> value
                SendAmountUnit.BTC -> value.multiply(SATS_PER_BTC)
            }.setScale(0, RoundingMode.DOWN)
        if (scaled > MAX_SATS) return ""
        val whole = scaled.longValueExact()
        return when (unit) {
            SendAmountUnit.USD -> usdString(whole)
            SendAmountUnit.SATS -> whole.toString()
            SendAmountUnit.BTC -> btcString(whole, trimTrailingZeros = true)
        }
    }

    /**
     * Sats this send moves: a fixed-amount invoice's amount, a BIP-21 amount when nothing was
     * typed, otherwise the typed amount in [unit]. Zero when there is no valid amount.
     */
    fun effectiveSats(
        destination: SendDestination?,
        inputText: String,
        unit: SendAmountUnit,
        btcPrice: Double,
    ): Long {
        when (destination) {
            is SendDestination.Bolt11 ->
                destination.amountMsat
                    ?.takeIf { it > 0 }
                    ?.let {
                        return it / 1000
                    }
            is SendDestination.Onchain ->
                destination.amountSats
                    ?.takeIf { it > 0 && inputText.isBlank() }
                    ?.let {
                        return it
                    }
            else -> Unit
        }
        val value = parseDecimal(inputText)?.takeIf { it.signum() > 0 } ?: return 0
        val sats =
            when (unit) {
                SendAmountUnit.SATS -> satsValue(value)
                SendAmountUnit.USD -> satsFromUsd(value, btcPrice)
                SendAmountUnit.BTC -> satsFromBtc(value)
            }
        return sats ?: 0
    }

    /** [sats] as text in [unit], truncated to its precision; BTC is exact, so it round-trips. */
    fun formatSats(sats: Long, unit: SendAmountUnit, btcPrice: Double): String {
        if (sats <= 0) return ""
        return when (unit) {
            SendAmountUnit.USD -> centsFromSats(sats, btcPrice)?.let(::usdString) ?: ""
            SendAmountUnit.SATS -> sats.toString()
            SendAmountUnit.BTC -> btcString(sats, trimTrailingZeros = false)
        }
    }

    /** Re-expresses typed text in another unit, keeping the underlying sats. */
    fun switchUnit(
        from: SendAmountUnit,
        to: SendAmountUnit,
        text: String,
        btcPrice: Double,
    ): String = formatSats(effectiveSats(null, text, from, btcPrice), to, btcPrice)

    /** [percent] of [balanceSats] as text in [unit], for the 25% and 50% shortcuts. */
    fun percentageOf(
        percent: Int,
        balanceSats: Long,
        unit: SendAmountUnit,
        btcPrice: Double,
    ): String {
        if (balanceSats <= 0 || btcPrice <= 0 || percent < 0) return ""
        val target =
            BigDecimal(balanceSats)
                .multiply(BigDecimal(percent))
                .divide(HUNDRED, 0, RoundingMode.DOWN)
        return formatSats(target.longValueExact(), unit, btcPrice)
    }
}
