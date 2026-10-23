package com.stablechannels.app.ui.transfer

import com.stablechannels.app.util.Constants
import kotlin.math.floor

/** The Lightning "Send Max" amount, as iOS works it out (SendChannelSpendPolicy). */
internal object SendMax {
    /** LDK's default routing fee ceiling, 1% + 50 sats, rounded up to whole sats. */
    fun maxRoutingFeeSats(sats: Long): Long = (sats + 99) / 100 + 50

    /**
     * The largest amount that leaves room for the routing fee ceiling. It comes from the whole
     * channel balance, stable and native BTC alike, capped by the channels' outbound capacity.
     */
    fun lightningSats(lightningBalanceSats: Long, outboundCapacitySats: Long?): Long {
        val available = minOf(lightningBalanceSats, outboundCapacitySats ?: lightningBalanceSats)
        fun fits(sats: Long) = sats + maxRoutingFeeSats(sats) <= available
        var sats = maxOf((available - 50) * 100 / 101, 0)
        while (fits(sats + 1)) sats++
        while (sats > 0 && !fits(sats)) sats--
        return sats
    }

    /**
     * The on-chain "Send Max" amount when it splices out, as iOS works it out: one channel's
     * outbound capacity, capped by the channel balance, less the splice fee estimate.
     */
    fun spliceOutSats(lightningBalanceSats: Long, outboundCapacitySats: Long, feeRateSatVb: Long) =
        maxOf(
            minOf(lightningBalanceSats, outboundCapacitySats) -
                feeRateSatVb * Constants.ESTIMATED_SPLICE_OUT_VBYTES,
            0,
        )

    /** USD for [sats], rounded down to the cent so the amount sent stays within it. */
    fun usd(sats: Long, btcPrice: Double): Double =
        floor(sats.toDouble() / Constants.SATS_IN_BTC * btcPrice * 100) / 100
}
