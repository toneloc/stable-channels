package com.stablechannels.app.services

import com.stablechannels.app.util.Constants
import java.util.Locale
import kotlin.math.ceil
import kotlin.math.floor

/** Trade-entry only. Keep settlements/reconciliation and USD -> BTC reductions uncapped. */
object StabilizationPolicy {
    fun backingCap(postFeeSpendable: Long): Long? {
        if (postFeeSpendable < 0) return null
        // Quotient/remainder avoids overflowing even on malformed Long.MAX_VALUE inputs.
        val percent = (postFeeSpendable / 100) * Constants.MAX_STABLE_ALLOCATION_PERCENT +
            ((postFeeSpendable % 100) * Constants.MAX_STABLE_ALLOCATION_PERCENT) / 100
        return percent
    }

    fun clientLimit(postFeeSpendable: Long): Long? = backingCap(postFeeSpendable)
        ?.let { it - Constants.CLIENT_SAFETY_MARGIN_SATS }?.takeIf { it > 0L }

    fun maximumMessage(cents: Long) = String.format(Locale.US, "Maximum additional trade: $%.2f", cents / 100.0)

    fun limitExceededMessage(cents: Long) =
        maximumMessage(cents) + "\nKeeps a small BTC reserve in the channel."
}

class TradeValidationException(message: String) : IllegalArgumentException(message)

data class StabilizationSnapshot(
    val receiverSats: Long,
    val spendableSats: Long,
    val backingSats: Long,
    val expectedUsd: Double,
    val price: Double
) {
    fun accepts(orderCents: Long): Boolean {
        return fits(orderCents, false)
    }

    private fun fits(orderCents: Long, searching: Boolean): Boolean {
        if (orderCents <= 0 || receiverSats < 0 || spendableSats < 0 || backingSats < 0 ||
            !price.isFinite() || price <= 0 || !expectedUsd.isFinite() || expectedUsd < 0) return false
        val amount = orderCents / 100.0
        val required = ceil(amount / price * Constants.SATS_IN_BTC)
        if (!required.isFinite() || required > (receiverSats - backingSats).coerceAtLeast(0).toDouble()) return false
        val target = TradeProtocol.normalizeExpectedUsd(expectedUsd + (amount - amount * Constants.STABLE_CHANNEL_TRADE_FEE_RATE))
        if (target < expectedUsd || (!searching && target == expectedUsd)) return false
        val fee = (TradeProtocol.expectedTradeFeeMsat(expectedUsd, target, price) ?: return false) / 1000
        if (fee > receiverSats || fee > spendableSats) return false
        val limit = StabilizationPolicy.clientLimit(spendableSats - fee) ?: return false
        if (target > (receiverSats - fee).toDouble() / Constants.SATS_IN_BTC * price) return false
        // Search the upper bound without treating a one-cent no-op as proof that larger
        // orders cannot fit. Actual submission still requires a nonzero target/backing.
        if (searching && backingSats == 0L && floor(target / price * Constants.SATS_IN_BTC) ==
            floor(expectedUsd / price * Constants.SATS_IN_BTC)) return true
        val backing = TradeProtocol.tradeBackingAfterDelta(receiverSats - fee, backingSats, expectedUsd, target, price) ?: return false
        return backing <= limit
    }

    fun maxOrderCents(): Long {
        if (!price.isFinite() || price <= 0 || !expectedUsd.isFinite() || expectedUsd < 0 || receiverSats < 0 || backingSats < 0) return 0
        val cents = floor((receiverSats - backingSats).coerceAtLeast(0).toDouble() / Constants.SATS_IN_BTC * price * 100)
        if (!cents.isFinite() || cents < 1 || cents >= Long.MAX_VALUE.toDouble()) return 0
        var low = 0L
        var high = cents.toLong()
        while (low < high) {
            val distance = high - low
            val mid = low + distance / 2 + distance % 2
            if (fits(mid, true)) low = mid else high = mid - 1
        }
        return if (low > 0 && accepts(low)) low else 0
    }
}
