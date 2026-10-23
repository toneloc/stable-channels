package com.stablechannels.app.domain.send

/** Checks every send must pass before dispatch, whichever screen sends it. */
object SendPreflight {
    data class Input(
        val isOnline: Boolean,
        val isFeeRateReady: Boolean,
        val amountSats: Long,
        val totalDebitSats: Long,
        val availableSats: Long,
        /**
         * The on-chain wallet send-all spends whatever the wallet holds; the node rejects an empty
         * one.
         */
        val sendsAllFromOnchainWallet: Boolean,
    )

    /** Null when the send may proceed. */
    fun evaluate(input: Input): SendError? =
        when {
            !input.isOnline -> SendError.Offline
            !input.isFeeRateReady -> SendError.FeeRateNotReady
            input.sendsAllFromOnchainWallet -> null
            input.amountSats <= 0 -> SendError.InvalidAmount
            input.availableSats <= 0 || input.totalDebitSats > input.availableSats ->
                SendError.InsufficientBalance(input.availableSats)
            else -> null
        }
}
