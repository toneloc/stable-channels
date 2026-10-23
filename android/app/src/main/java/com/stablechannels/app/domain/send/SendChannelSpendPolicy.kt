package com.stablechannels.app.domain.send

/** The channel facts spend decisions need, built from the node's channel list at the call site. */
data class SpliceChannelCandidate(
    val channelId: String,
    val userChannelId: String,
    val counterpartyNodeId: String,
    val outboundCapacityMsat: Long,
    val isReady: Boolean,
) {
    val outboundCapacitySats: Long
        get() = outboundCapacityMsat / 1000
}

/** What a send can spend and which channel funds an on-chain splice-out. */
object SendChannelSpendPolicy {
    /**
     * Sats spendable toward [destination]. Lightning is bounded by ready channels' outbound
     * capacity; on-chain follows [availableOnchainSpendableSats]. Without a destination, the total
     * balance.
     */
    fun availableSpendableSats(
        destination: SendDestination?,
        channels: List<SpliceChannelCandidate>,
        lightningBalanceSats: Long,
        onchainBalanceSats: Long,
        totalBalanceSats: Long,
        isSweeping: Boolean,
    ): Long {
        destination ?: return totalBalanceSats
        if (destination is SendDestination.Onchain) {
            return availableOnchainSpendableSats(
                channels,
                lightningBalanceSats,
                onchainBalanceSats,
                isSweeping,
            )
        }
        val ready = channels.filter { it.isReady }
        if (ready.isEmpty()) return lightningBalanceSats
        return minOf(ready.sumOf { it.outboundCapacityMsat } / 1000, lightningBalanceSats)
    }

    /**
     * A splice-out runs against one channel, so capacity is not summed across channels: it is
     * bounded by the largest ready channel and the Lightning balance. Without a ready channel, or
     * during a sweep, the on-chain wallet pays.
     */
    fun availableOnchainSpendableSats(
        channels: List<SpliceChannelCandidate>,
        lightningBalanceSats: Long,
        onchainBalanceSats: Long,
        isSweeping: Boolean,
    ): Long {
        val ready = channels.filter { it.isReady }
        if (ready.isEmpty() || isSweeping) return onchainBalanceSats
        return minOf(ready.maxOf { it.outboundCapacitySats }, lightningBalanceSats)
    }

    /**
     * Channel for a splice-out of [amountSats] plus its fee at [feeRateSatVb]: the smallest ready
     * channel that covers both, or the largest ready channel when none does.
     */
    fun selectSpliceChannel(
        channels: List<SpliceChannelCandidate>,
        amountSats: Long,
        feeRateSatVb: Double,
    ): SpliceChannelCandidate? {
        val fee = PaymentFeeEstimator.estimateSpliceOutFee(feeRateSatVb)
        val required = PaymentFeeEstimator.saturatingAdd(amountSats, fee)
        val ready = channels.filter { it.isReady }
        return ready
            .filter { it.outboundCapacitySats >= required }
            .minByOrNull { it.outboundCapacityMsat }
            ?: ready.maxByOrNull { it.outboundCapacityMsat }
    }

    /** True when an on-chain send goes out as a splice-out from an open channel. */
    fun isSpliceOut(channels: List<SpliceChannelCandidate>, isSweeping: Boolean): Boolean =
        !isSweeping && channels.any { it.isReady }

    /**
     * A Max send to an on-chain address with no splice-out empties the on-chain wallet in one
     * transaction that pays its fee from the balance.
     */
    fun isOnchainSendAll(
        isMax: Boolean,
        destination: SendDestination?,
        channels: List<SpliceChannelCandidate>,
        isSweeping: Boolean,
    ): Boolean =
        isMax && destination is SendDestination.Onchain && !isSpliceOut(channels, isSweeping)

    /**
     * Largest amount whose amount plus [feeEstimator] fee fits in [available]. [principalCap] (an
     * LNURL payee's `maxSendable`) bounds the amount only; the fee is still paid from [available].
     * Assumes the fee never decreases as the amount grows.
     */
    fun calculateMaxSendable(
        available: Long,
        principalCap: Long = Long.MAX_VALUE,
        feeEstimator: (Long) -> Long,
    ): Long {
        if (available <= 0 || principalCap <= 0) return 0
        fun fits(amount: Long): Boolean {
            val fee = feeEstimator(amount)
            return fee <= available && amount <= available - fee
        }

        val capped = minOf(principalCap, available)
        if (fits(capped)) return capped

        // Fixed-point iteration converges fast for proportional fees; the linear walk finishes it.
        var candidate = available
        repeat(8) { candidate = (available - feeEstimator(candidate)).coerceAtLeast(0) }
        var steps = 0
        while (candidate > 0 && !fits(candidate) && steps < 64) {
            candidate--
            steps++
        }
        return if (fits(candidate)) minOf(candidate, principalCap) else 0
    }
}
