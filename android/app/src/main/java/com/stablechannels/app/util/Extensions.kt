package com.stablechannels.app.util

import java.text.NumberFormat
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.TimeUnit

fun Long.satsFormatted(): String {
    val nf = NumberFormat.getNumberInstance(Locale.US)
    return "${nf.format(this)} sats"
}

fun Long.btcFormatted(): String {
    val btc = this.toDouble() / Constants.SATS_IN_BTC
    return String.format(Locale.US, "%.8f BTC", btc)
}

fun Double.usdFormatted(): String {
    val nf = NumberFormat.getCurrencyInstance(Locale.US)
    return nf.format(this)
}

fun Date.relativeString(): String {
    val now = System.currentTimeMillis()
    val diff = now - this.time
    return when {
        diff < TimeUnit.MINUTES.toMillis(1) -> "just now"
        diff < TimeUnit.HOURS.toMillis(1) -> "${TimeUnit.MILLISECONDS.toMinutes(diff)}m ago"
        diff < TimeUnit.DAYS.toMillis(1) -> "${TimeUnit.MILLISECONDS.toHours(diff)}h ago"
        diff < TimeUnit.DAYS.toMillis(7) -> "${TimeUnit.MILLISECONDS.toDays(diff)}d ago"
        else -> {
            val sdf = SimpleDateFormat("MMM d", Locale.US)
            sdf.format(this)
        }
    }
}

fun Date.shortString(): String {
    val sdf = SimpleDateFormat("MMM d, h:mm a", Locale.US)
    return sdf.format(this)
}

fun Long.toDate(): Date = Date(this * 1000)
