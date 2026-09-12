package com.stablechannels.app.util

import java.text.NumberFormat
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.TimeUnit

internal object AppFormatters {
    private val numberInstance: ThreadLocal<NumberFormat> = ThreadLocal.withInitial {
        NumberFormat.getNumberInstance(Locale.US)
    }

    private val currencyInstance: ThreadLocal<NumberFormat> = ThreadLocal.withInitial {
        NumberFormat.getCurrencyInstance(Locale.US)
    }

    private val relativeDayMonth: ThreadLocal<SimpleDateFormat> = ThreadLocal.withInitial {
        SimpleDateFormat("MMM d", Locale.US)
    }

    private val shortDateTime: ThreadLocal<SimpleDateFormat> = ThreadLocal.withInitial {
        SimpleDateFormat("MMM d, h:mm a", Locale.US)
    }

    fun formatSats(amount: Long): String {
        return numberInstance.get()?.format(amount) ?: amount.toString()
    }

    fun formatUsd(amount: Double): String {
        return currencyInstance.get()?.format(amount) ?: "$0.00"
    }

    fun formatRelativeDayMonth(date: Date): String {
        val formatter = relativeDayMonth.get() ?: return ""
        formatter.timeZone = TimeZone.getDefault()
        return formatter.format(date)
    }

    fun formatShortDateTime(date: Date): String {
        val formatter = shortDateTime.get() ?: return ""
        formatter.timeZone = TimeZone.getDefault()
        return formatter.format(date)
    }
}

fun Long.satsFormatted(): String {
    return AppFormatters.formatSats(this)
}

fun Long.btcFormatted(): String {
    val btc = this.toDouble() / Constants.SATS_IN_BTC
    return String.format(Locale.US, "%.8f BTC", btc)
}

/**
 * Format as BTC with spaced digit groups: "0.00 190 079" (matches iOS).
 * Preallocates buffer capacity to minimize intermediate allocations.
 */
fun Long.btcSpacedFormatted(): String {
    val btc = this.toDouble() / Constants.SATS_IN_BTC
    val raw = String.format(Locale.US, "%.8f", btc)
    val dotIndex = raw.indexOf('.')
    if (dotIndex < 0) return raw

    val whole = raw.substring(0, dotIndex)
    val decimals = raw.substring(dotIndex + 1)
    if (decimals.length < 8) return "$whole.$decimals"

    val sb = java.lang.StringBuilder(raw.length + 4)
    sb.append(whole).append('.')
    sb.append(decimals, 0, 2)
    sb.append('\u2009')
    sb.append(decimals, 2, 5)
    sb.append('\u2009')
    sb.append(decimals, 5, 8)
    return sb.toString()
}

fun Double.usdFormatted(): String {
    return AppFormatters.formatUsd(this)
}

fun Date.relativeString(): String {
    val now = System.currentTimeMillis()
    val diff = now - this.time
    return when {
        diff < TimeUnit.MINUTES.toMillis(1) -> "just now"
        diff < TimeUnit.HOURS.toMillis(1) -> "${TimeUnit.MILLISECONDS.toMinutes(diff)}m ago"
        diff < TimeUnit.DAYS.toMillis(1) -> "${TimeUnit.MILLISECONDS.toHours(diff)}h ago"
        diff < TimeUnit.DAYS.toMillis(7) -> "${TimeUnit.MILLISECONDS.toDays(diff)}d ago"
        else -> AppFormatters.formatRelativeDayMonth(this)
    }
}

fun Date.shortString(): String {
    return AppFormatters.formatShortDateTime(this)
}

fun Long.toDate(): Date = Date(this * 1000)
