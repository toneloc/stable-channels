package com.stablechannels.app.domain.send

import com.stablechannels.app.util.Constants
import kotlin.math.ceil

/** Lightning routing-fee ceilings and on-chain fee estimates. Amounts are non-negative. */
object PaymentFeeEstimator {
    /** LDK's default `max_total_routing_fee_msat`: 1% of the amount plus 50 sats. */
    fun maxLightningRoutingFeeMsat(amountMsat: Long): Long = saturatingAdd(amountMsat / 100, 50_000)

    /** The routing-fee ceiling in whole sats, rounded up so what is shown always covers it. */
    fun maxLightningRoutingFeeSats(sats: Long): Long =
        maxLightningRoutingFeeSatsForMsat(saturatingMultiply(sats, 1_000))

    /** The ceiling for an exact msat amount (a fixed-amount invoice), rounded up to whole sats. */
    fun maxLightningRoutingFeeSatsForMsat(amountMsat: Long): Long {
        if (amountMsat <= 0) return 0
        return roundUpToSats(maxLightningRoutingFeeMsat(amountMsat))
    }

    /** Most an exact msat Lightning payment can debit (amount plus fee ceiling), in whole sats. */
    fun maxLightningDebitSats(amountMsat: Long): Long {
        if (amountMsat <= 0) return 0
        return roundUpToSats(saturatingAdd(amountMsat, maxLightningRoutingFeeMsat(amountMsat)))
    }

    /**
     * Converts sat/vB to the sat/kwu LDK broadcasts with (1 vB = 4 WU), rounded up so a fractional
     * rate such as 1.28 sat/vB is never sent below the rate shown.
     */
    fun satPerKwu(satPerVb: Double): Long =
        if (satPerVb.isFinite() && satPerVb > 0) ceil(satPerVb * 250).toLong() else 0

    /** Expected fee of an on-chain wallet send; a send-all spends every UTXO, so it is larger. */
    fun estimateOnchainFee(
        feeRateSatVb: Double,
        isSendAll: Boolean,
        sendVBytes: Long = Constants.ESTIMATED_ONCHAIN_SEND_VBYTES,
        sendAllVBytes: Long = Constants.ESTIMATED_ONCHAIN_SEND_ALL_VBYTES,
    ): Long = feeForSize(feeRateSatVb, if (isSendAll) sendAllVBytes else sendVBytes)

    /** Expected fee of a splice-out from a channel to an on-chain address. */
    fun estimateSpliceOutFee(
        feeRateSatVb: Double,
        vbytes: Long = Constants.ESTIMATED_SPLICE_OUT_VBYTES,
    ): Long = feeForSize(feeRateSatVb, vbytes)

    private fun feeForSize(feeRateSatVb: Double, vbytes: Long): Long {
        if (!feeRateSatVb.isFinite() || feeRateSatVb <= 0) return 0
        val raw = vbytes.toDouble() * feeRateSatVb
        return if (raw >= Long.MAX_VALUE.toDouble()) Long.MAX_VALUE else ceil(raw).toLong()
    }

    private fun roundUpToSats(msat: Long): Long = msat / 1_000 + if (msat % 1_000 == 0L) 0 else 1

    fun saturatingAdd(a: Long, b: Long): Long {
        val sum = a + b
        return if (((a xor sum) and (b xor sum)) < 0) Long.MAX_VALUE else sum
    }

    fun saturatingMultiply(a: Long, b: Long): Long =
        if (a != 0L && b > Long.MAX_VALUE / a) Long.MAX_VALUE else a * b
}
