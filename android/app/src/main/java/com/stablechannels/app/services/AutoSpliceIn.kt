package com.stablechannels.app.services

/**
 * When on-chain funds move into the channel on their own, as Phoenix does (#445): once every
 * confirmed wallet transaction is 3 blocks deep, and only while the splice fee stays under the cap.
 * Funds held back by the fee cap can still be moved by hand with Move.
 */
object AutoSpliceIn {
    const val MIN_CONFIRMATIONS = 3

    /** Phoenix's default fee cap: 5,000 sats, and no more than half the amount. */
    const val MAX_FEE_SATS = 5_000L
    const val MAX_FEE_PERCENT = 50L

    /** Splice-in size: the shared channel input and output plus one wallet input. */
    const val ESTIMATED_VBYTES = 170L

    /**
     * At most one automatic attempt per 10 minutes, so a failing splice isn't retried each tick.
     */
    const val RETRY_AFTER_MS = 10 * 60 * 1000L

    enum class Step {
        IDLE,
        CONFIRMING,
        RETRY_LATER,
        FEE_TOO_HIGH,
        SPLICE,
    }

    /**
     * [newestConfirmations] is null when no confirmed wallet transaction is known, which doesn't
     * block. It and [feeRateSatVb] are only called when needed: the fee rate is a network request.
     */
    fun decide(
        canSplice: Boolean,
        spendableSats: Long,
        newestConfirmations: () -> Int?,
        lastAttemptAtMs: Long?,
        nowMs: Long,
        feeRateSatVb: () -> Long?,
    ): Step {
        if (!canSplice || spendableSats <= 0) return Step.IDLE
        val confirmations = newestConfirmations()
        if (confirmations != null && confirmations < MIN_CONFIRMATIONS) return Step.CONFIRMING
        if (lastAttemptAtMs != null && nowMs - lastAttemptAtMs < RETRY_AFTER_MS) {
            return Step.RETRY_LATER
        }
        val feeSats = (feeRateSatVb() ?: return Step.RETRY_LATER) * ESTIMATED_VBYTES
        return if (feeSats > MAX_FEE_SATS || feeSats * 100 > spendableSats * MAX_FEE_PERCENT) {
            Step.FEE_TOO_HIGH
        } else {
            Step.SPLICE
        }
    }

    /** Confirmations of the newest of [confirmedHeights], or null when there are none. */
    fun newestConfirmations(confirmedHeights: List<Int>, tipHeight: Int): Int? =
        confirmedHeights.maxOrNull()?.let { maxOf(tipHeight - it + 1, 0) }
}
