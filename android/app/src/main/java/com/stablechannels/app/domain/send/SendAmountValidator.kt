package com.stablechannels.app.domain.send

/** Balances and channels at the moment a send is priced. */
data class SendBalanceSnapshot(
    val channels: List<SpliceChannelCandidate>,
    val lightningSats: Long,
    val onchainSats: Long,
    val totalSats: Long,
    val isSweeping: Boolean,
)

/** An LNURL payee's accepted amount range. */
data class PayeeAmountLimits(val minSats: Long, val maxSats: Long)

/** What the user asked to send. */
data class SendAmountRequest(
    val destination: SendDestination,
    /** The typed or preset amount, from [SendAmountCalculator.effectiveSats]. */
    val enteredSats: Long,
    /** The fee rate of the selected tier, in sat/vB. */
    val feeRateSatVb: Double,
    /** Max: send the largest amount that fits after the fee. */
    val isMax: Boolean = false,
    /** Settings "Send All": empty the on-chain wallet even when a channel is ready. */
    val sendsAllFromOnchainWallet: Boolean = false,
    val payeeLimits: PayeeAmountLimits? = null,
    /** A fetched LNURL invoice fixes the amount, so what is shown, checked and paid match. */
    val quotedAmountSats: Long? = null,
)

/** The priced send: what moves, what it costs and what the wallet can cover. */
data class SendQuote(
    val amountSats: Long,
    val feeSats: Long,
    val totalDebitSats: Long,
    val availableSats: Long,
    val isSpliceOut: Boolean,
    /** Max to an on-chain address without a splice-out: one transaction empties the wallet. */
    val sendsEntireOnchainBalance: Boolean,
)

/** Prices a send against a balance snapshot and decides whether it can go ahead. */
object SendAmountValidator {
    fun quote(request: SendAmountRequest, balances: SendBalanceSnapshot): SendQuote {
        val destination = request.destination
        val available =
            if (request.sendsAllFromOnchainWallet) balances.onchainSats
            else
                SendChannelSpendPolicy.availableSpendableSats(
                    destination,
                    balances.channels,
                    balances.lightningSats,
                    balances.onchainSats,
                    balances.totalSats,
                    balances.isSweeping,
                )
        val isSpliceOut =
            !request.sendsAllFromOnchainWallet &&
                SendChannelSpendPolicy.isSpliceOut(balances.channels, balances.isSweeping)

        fun feeFor(sats: Long): Long =
            when {
                destination.isLightning -> PaymentFeeEstimator.maxLightningRoutingFeeSats(sats)
                isSpliceOut -> PaymentFeeEstimator.estimateSpliceOutFee(request.feeRateSatVb)
                else ->
                    PaymentFeeEstimator.estimateOnchainFee(
                        request.feeRateSatVb,
                        isSendAll = request.isMax || request.sendsAllFromOnchainWallet,
                    )
            }

        val amount =
            when {
                request.quotedAmountSats != null -> request.quotedAmountSats
                request.sendsAllFromOnchainWallet ->
                    (available - feeFor(available)).coerceAtLeast(0)
                request.isMax ->
                    SendChannelSpendPolicy.calculateMaxSendable(
                        available,
                        request.payeeLimits?.maxSats ?: Long.MAX_VALUE,
                        ::feeFor,
                    )
                else -> request.enteredSats
            }

        // A fixed-amount invoice is priced from its exact msat so no sub-sat remainder escapes the
        // ceiling.
        val fixedMsat = (destination as? SendDestination.Bolt11)?.amountMsat?.takeIf { it > 0 }
        val fee =
            fixedMsat?.let(PaymentFeeEstimator::maxLightningRoutingFeeSatsForMsat) ?: feeFor(amount)

        return SendQuote(
            amountSats = amount,
            feeSats = fee,
            totalDebitSats =
                when {
                    request.sendsAllFromOnchainWallet -> available
                    // Round the amount and fee up together, as the node debits whole msat.
                    fixedMsat != null -> PaymentFeeEstimator.maxLightningDebitSats(fixedMsat)
                    else -> PaymentFeeEstimator.saturatingAdd(amount, fee)
                },
            availableSats = available,
            isSpliceOut = isSpliceOut,
            sendsEntireOnchainBalance =
                SendChannelSpendPolicy.isOnchainSendAll(
                    request.isMax,
                    destination,
                    balances.channels,
                    balances.isSweeping,
                ),
        )
    }

    /** Null when the amount can be sent; otherwise the first rule it breaks. */
    fun validate(request: SendAmountRequest, quote: SendQuote): SendError? {
        val limits = request.payeeLimits
        return when {
            // The send-all spends whatever the wallet holds; the node rejects an empty wallet.
            request.sendsAllFromOnchainWallet -> null
            quote.availableSats <= 0 -> SendError.InsufficientBalance(0)
            quote.amountSats <= 0 -> SendError.InvalidAmount
            limits != null && quote.amountSats !in limits.minSats..limits.maxSats ->
                SendError.OutsidePayeeLimits(limits.minSats, limits.maxSats)
            quote.amountSats > quote.availableSats ->
                SendError.AmountExceedsBalance(quote.availableSats)
            quote.totalDebitSats > quote.availableSats ->
                SendError.InsufficientBalance(quote.availableSats)
            else -> null
        }
    }
}
