package com.stablechannels.app.util

import android.content.Context

/**
 * Persists the most recent accepted oracle price so the background stability service can apply
 * the same large-move circuit breaker as the foreground app. Port of the iOS app-group
 * `PriceOracleAnchorStore`: without an anchor, `PriceOracle.resolve` receives a null
 * `lastTrustedPrice` and a manipulated-but-internally-consistent price would be accepted.
 */
object PriceOracleAnchorStore {
    private const val PREFS_NAME = "price_oracle_anchor_v1"
    private const val PRICE_BITS_KEY = "price_bits"
    private const val ACCEPTED_AT_MS_KEY = "accepted_at_ms"

    fun save(context: Context, price: Double, acceptedAtMs: Long = System.currentTimeMillis()) {
        if (!PriceOracle.isPlausibleBitcoinPrice(price)) return
        context.applicationContext
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            // Doubles are stored as raw bits: SharedPreferences only offers lossy putFloat.
            .putLong(PRICE_BITS_KEY, price.toRawBits())
            .putLong(ACCEPTED_AT_MS_KEY, acceptedAtMs)
            .apply()
    }

    fun freshPrice(context: Context, nowMs: Long = System.currentTimeMillis()): Double? {
        val prefs = context.applicationContext
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        if (!prefs.contains(PRICE_BITS_KEY) || !prefs.contains(ACCEPTED_AT_MS_KEY)) return null
        return freshPrice(
            Double.fromBits(prefs.getLong(PRICE_BITS_KEY, 0L)),
            prefs.getLong(ACCEPTED_AT_MS_KEY, 0L),
            nowMs
        )
    }

    /** Pure freshness rule (unit-testable): plausible price whose age is within [0, max]. */
    fun freshPrice(price: Double, acceptedAtMs: Long, nowMs: Long): Double? {
        if (!PriceOracle.isPlausibleBitcoinPrice(price)) return null
        val ageMs = nowMs - acceptedAtMs
        if (ageMs < 0 || ageMs > PriceOracle.MAXIMUM_TRUSTED_PRICE_AGE_SECS * 1000) return null
        return price
    }
}
