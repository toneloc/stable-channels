package com.stablechannels.app.domain.send

import com.stablechannels.app.util.Constants

/** Confirmation speed the user picks for an on-chain send. Display labels live in the UI. */
enum class NetworkFeeSpeedTier {
    ECONOMY,
    STANDARD,
    PRIORITY;

    /**
     * Fee rate in sat/vB for this tier: the live [fees] when available, otherwise an estimate
     * derived from [baseRate] and clamped to the allowed range.
     */
    fun effectiveRate(baseRate: Double, fees: RecommendedFees? = null): Double {
        if (fees != null) return fees.rate(this)
        val floor = Constants.MIN_FEE_RATE_SAT_VB
        val ceiling = Constants.MAX_FEE_RATE_SAT_VB
        // NaN and infinite inputs fall back to the floor; zero and negatives clamp up to it.
        val normalized = if (baseRate.isFinite()) baseRate.coerceIn(floor, ceiling) else floor
        return when (this) {
            ECONOMY -> maxOf(floor, normalized * 0.8)
            STANDARD -> normalized
            // The step keeps priority strictly above standard at the floor (1.3 vs 1.0).
            PRIORITY ->
                minOf(ceiling, maxOf(normalized + PRIORITY_MINIMUM_STEP_SAT_VB, normalized * 1.3))
        }
    }

    private companion object {
        /** Minimum gap between priority and standard; a step, not a fee-rate floor. */
        const val PRIORITY_MINIMUM_STEP_SAT_VB = 0.1
    }
}

/**
 * Recommended fee rates in sat/vB. Every rate is sanitized on construction: non-finite or
 * non-positive values fall back to the minimum fee, and all are clamped to the allowed range.
 */
class RecommendedFees(
    fastestFee: Double,
    halfHourFee: Double,
    hourFee: Double,
    economyFee: Double? = null,
    minimumFee: Double,
) {
    val minimumFee: Double = sanitize(minimumFee, Constants.MIN_FEE_RATE_SAT_VB)
    val fastestFee: Double = sanitize(fastestFee, this.minimumFee)
    val halfHourFee: Double = sanitize(halfHourFee, this.minimumFee)
    val hourFee: Double = sanitize(hourFee, this.minimumFee)
    val economyFee: Double? = economyFee?.let { sanitize(it, this.minimumFee) }

    /** Tier rates never decrease from economy to standard to priority. */
    fun rate(tier: NetworkFeeSpeedTier): Double {
        val max = Constants.MAX_FEE_RATE_SAT_VB
        val minRate = maxOf(Constants.MIN_FEE_RATE_SAT_VB, minOf(minimumFee, halfHourFee, max))
        val economy = minOf(max, maxOf(minRate, economyFee?.takeIf { it > 0 } ?: hourFee))
        val standard = minOf(max, maxOf(economy, minRate, halfHourFee))
        val priority = minOf(max, maxOf(standard, minRate, fastestFee))
        return when (tier) {
            NetworkFeeSpeedTier.ECONOMY -> economy
            NetworkFeeSpeedTier.STANDARD -> standard
            NetworkFeeSpeedTier.PRIORITY -> priority
        }
    }

    override fun equals(other: Any?): Boolean =
        other is RecommendedFees &&
            fastestFee == other.fastestFee &&
            halfHourFee == other.halfHourFee &&
            hourFee == other.hourFee &&
            economyFee == other.economyFee &&
            minimumFee == other.minimumFee

    override fun hashCode(): Int =
        listOf(fastestFee, halfHourFee, hourFee, economyFee, minimumFee).hashCode()

    override fun toString(): String =
        "RecommendedFees(fastest=$fastestFee, halfHour=$halfHourFee, hour=$hourFee, " +
            "economy=$economyFee, minimum=$minimumFee)"

    companion object {
        fun sanitize(rate: Double, fallback: Double): Double {
            if (!rate.isFinite() || rate <= 0) return fallback
            return rate.coerceIn(Constants.MIN_FEE_RATE_SAT_VB, Constants.MAX_FEE_RATE_SAT_VB)
        }
    }
}
