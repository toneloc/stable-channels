package com.stablechannels.app.ui.transfer.send

import androidx.lifecycle.viewModelScope
import com.stablechannels.app.domain.lnurl.LnurlInvoiceQuote
import com.stablechannels.app.domain.send.PaymentSettlement
import com.stablechannels.app.domain.send.SendDestination
import com.stablechannels.app.domain.send.SendError
import com.stablechannels.app.domain.send.SendFlowStep
import com.stablechannels.app.domain.send.SendPaymentOutcome
import com.stablechannels.app.domain.send.SendPaymentResult
import com.stablechannels.app.domain.send.SendPreflight
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.isActive
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A pending Lightning receipt is checked again for this long per round. */
internal const val PENDING_SETTLEMENT_ROUND_MS = 60_000L

/** The slide: sends the confirmed payment once, however often it is triggered. */
fun SendFlowModel.send() {
    viewModelScope.launch { executeSend() }
}

internal suspend fun SendFlowModel.executeSend() {
    val s = state.value
    if (s.isSending) return
    val destination = s.destination ?: return update { it.copy(resetToken = it.resetToken + 1) }
    val lnurlQuote =
        if (s.isLnurl) {
            if (lnurlClient() == null) return failAndReset(SendError.WalletNotReady)
            payableLnurlQuote() ?: return update { it.copy(resetToken = it.resetToken + 1) }
        } else null

    // Marked in flight before the first suspension, so a second slide cannot dispatch twice.
    update { it.copy(isSending = true, error = null) }
    try {
        val quote = quote() ?: return
        val preflight =
            SendPreflight.evaluate(
                SendPreflight.Input(
                    isOnline = wallet.isOnline,
                    isFeeRateReady = state.value.isFeeRateReady,
                    amountSats = quote.amountSats,
                    totalDebitSats = quote.totalDebitSats,
                    availableSats = quote.availableSats,
                    sendsAllFromOnchainWallet = state.value.sendsAllFromOnchainWallet,
                )
            )
        if (preflight != null) return fail(preflight)
        if (!authenticator.authorize(destination)) return fail(SendError.AuthenticationRequired)

        ensureLspConnected()
        val sendsAll = state.value.sendsAllFromOnchainWallet || quote.sendsEntireOnchainBalance
        val feeRate = state.value.effectiveFeeRateSatVb
        // Taken before dispatch: a settlement observed after this moment belongs to this attempt.
        val attemptStartedAt = nanoTime()
        val result =
            try {
                withContext(io) {
                    if (sendsAll && destination is SendDestination.Onchain) {
                        executor.sendAllOnchain(destination.address, feeRate)
                    } else {
                        executor.execute(destination, quote.amountSats, feeRate, lnurlQuote)
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e.toSendError())
                refreshLnurlQuoteAfterFailure()
                return
            }
        // The send-all shows what the recipient gets: the balance less the fee.
        val sentSats = if (sendsAll) quote.amountSats else result.sentAmountSats
        showOutcome(result, sentSats, destination, attemptStartedAt)
    } finally {
        update { it.copy(isSending = false, resetToken = it.resetToken + 1) }
    }
}

private suspend fun SendFlowModel.showOutcome(
    result: SendPaymentResult,
    sentSats: Long,
    destination: SendDestination,
    attemptStartedAt: Long,
) {
    when (val outcome = result.outcome) {
        // Already in the mempool.
        is SendPaymentOutcome.Onchain ->
            showReceipt(SendReceipt(sentSats, txid = outcome.txid, isPendingSettlement = false))
        // Confirms on-chain; there is no payment to await and no txid until negotiation ends.
        SendPaymentOutcome.SpliceOut ->
            showReceipt(SendReceipt(sentSats, isPendingSettlement = true))
        is SendPaymentOutcome.Lightning -> {
            val timeoutMs = if (destination is SendDestination.Bolt12) 10_000L else 7_000L
            val settled = settlement.await(outcome.paymentId, attemptStartedAt, timeoutMs)
            applySettlement(settled, sentSats, outcome.paymentId, attemptStartedAt)
        }
    }
}

/** A timeout is not a settlement: the receipt stays pending and keeps watching. */
internal fun SendFlowModel.applySettlement(
    settled: PaymentSettlement,
    sentSats: Long,
    paymentId: String,
    attemptStartedAt: Long,
) {
    when (settled) {
        PaymentSettlement.Settled ->
            showReceipt(SendReceipt(sentSats, paymentId = paymentId, isPendingSettlement = false))
        is PaymentSettlement.Failed -> {
            fail(SendError.PaymentFailed(settled.reason))
            refreshLnurlQuoteAfterFailure()
        }
        PaymentSettlement.TimedOut -> {
            showReceipt(SendReceipt(sentSats, paymentId = paymentId, isPendingSettlement = true))
            watchPendingSettlement(paymentId, attemptStartedAt)
        }
    }
}

/**
 * Follows a pending receipt until the payment resolves: settled completes it, failed returns to
 * confirm with the reason. Stops when the flow resets or the screen's model is cleared.
 */
private fun SendFlowModel.watchPendingSettlement(paymentId: String, attemptStartedAt: Long) {
    pendingSettlementJob?.cancel()
    pendingSettlementJob = viewModelScope.launch {
        while (isActive && state.value.isWatching(paymentId)) {
            val settled = settlement.await(paymentId, attemptStartedAt, PENDING_SETTLEMENT_ROUND_MS)
            if (!state.value.isWatching(paymentId)) return@launch
            when (settled) {
                PaymentSettlement.Settled ->
                    update { s ->
                        s.copy(receipt = s.receipt?.copy(isPendingSettlement = false))
                    }
                is PaymentSettlement.Failed -> {
                    update { it.copy(receipt = null) }
                    fail(SendError.PaymentFailed(settled.reason))
                    goTo(SendFlowStep.CONFIRM)
                    refreshLnurlQuoteAfterFailure()
                }
                PaymentSettlement.TimedOut -> Unit
            }
        }
    }
}

private fun SendFlowState.isWatching(paymentId: String): Boolean =
    step == SendFlowStep.SUCCESS &&
        receipt?.isPendingSettlement == true &&
        receipt.paymentId == paymentId

private fun SendFlowModel.showReceipt(receipt: SendReceipt) {
    update { it.copy(receipt = receipt) }
    goTo(SendFlowStep.SUCCESS)
}

private fun SendFlowModel.failAndReset(error: SendError) = update {
    it.copy(error = error, resetToken = it.resetToken + 1)
}

/** Fetches the confirm step's invoice unless one is shown or a fetch is running. */
internal fun SendFlowModel.loadLnurlQuoteIfNeeded() {
    val s = state.value
    if (s.step != SendFlowStep.CONFIRM || !s.isLnurl) return
    if (s.lnurlQuote != null || lnurlQuoteJob != null) return
    startLnurlQuoteFetch()
}

/** Replaces any quote with a fresh fetch; a replaced fetch is cancelled, so it never lands. */
internal fun SendFlowModel.startLnurlQuoteFetch() {
    invalidateLnurlQuote()
    val client =
        lnurlClient() ?: return update { it.copy(lnurlQuoteError = SendError.WalletNotReady) }
    val params =
        state.value.lnurlParams
            ?: return update { it.copy(lnurlQuoteError = SendError.MissingLnurlInvoice) }
    val amountMsat = (quote()?.amountSats ?: 0) * 1000
    val comment = LnurlInvoiceQuote.normalizedComment(state.value.lnurlComment)
    update { it.copy(isFetchingLnurlQuote = true) }
    val job =
        viewModelScope.launch(start = CoroutineStart.LAZY) {
            try {
                val quote = withContext(io) { client.fetchInvoice(params, amountMsat, comment) }
                update { it.copy(lnurlQuote = quote) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                update { it.copy(lnurlQuoteError = e.toSendError()) }
            } finally {
                // A replaced or invalidated fetch no longer owns this state.
                if (lnurlQuoteJob === coroutineContext.job) {
                    update { it.copy(isFetchingLnurlQuote = false) }
                    lnurlQuoteJob = null
                }
            }
        }
    lnurlQuoteJob = job
    job.start()
}

/** After a failed LNURL payment a retry must pay a fresh invoice, never the failed one. */
internal fun SendFlowModel.refreshLnurlQuoteAfterFailure() {
    if (state.value.isLnurl) startLnurlQuoteFetch()
}

/** Drops the quote and abandons any fetch in flight. */
internal fun SendFlowModel.invalidateLnurlQuote() {
    lnurlQuoteJob?.cancel()
    lnurlQuoteJob = null
    update { it.copy(lnurlQuote = null, lnurlQuoteError = null, isFetchingLnurlQuote = false) }
}

/**
 * The quote the slide may pay. An expired one is replaced and must be reviewed: never auto-paid.
 */
private fun SendFlowModel.payableLnurlQuote(): LnurlInvoiceQuote? {
    val quote = state.value.lnurlQuote ?: return null
    if (!quote.isExpired(nowEpochSecs())) return quote
    startLnurlQuoteFetch()
    fail(SendError.LnurlInvoiceRefreshed)
    return null
}
