package com.stablechannels.app.ui.transfer.send

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.stablechannels.app.domain.lnurl.LnurlError
import com.stablechannels.app.domain.lnurl.LnurlPayClient
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DestinationClassification
import com.stablechannels.app.domain.send.InvoicePayability
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import com.stablechannels.app.domain.send.PayeeAmountLimits
import com.stablechannels.app.domain.send.PaymentDestinationClassifier
import com.stablechannels.app.domain.send.PaymentSettlementWatcher
import com.stablechannels.app.domain.send.SendAmountCalculator
import com.stablechannels.app.domain.send.SendAmountRequest
import com.stablechannels.app.domain.send.SendAmountUnit
import com.stablechannels.app.domain.send.SendAmountValidator
import com.stablechannels.app.domain.send.SendAuthenticator
import com.stablechannels.app.domain.send.SendDestination
import com.stablechannels.app.domain.send.SendError
import com.stablechannels.app.domain.send.SendFailure
import com.stablechannels.app.domain.send.SendFlowStep
import com.stablechannels.app.domain.send.SendPaymentDispatcher
import com.stablechannels.app.domain.send.SendQuote
import com.stablechannels.app.domain.send.SendWallet
import com.stablechannels.app.domain.send.presetSats
import com.stablechannels.app.util.epochSecondsNow
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * State and orchestration for the send flow: recipient, amount, confirm, then the receipt. Pricing
 * and rules live in the domain; this model only sequences them. Sending is in SendFlowExecution.kt.
 */
class SendFlowModel(
    internal val wallet: SendWallet,
    internal val executor: SendPaymentDispatcher,
    internal val settlement: PaymentSettlementWatcher,
    private val classifier: PaymentDestinationClassifier,
    private val decoder: LightningInvoiceDecoder,
    private val lnurlClientFor: (BitcoinNetwork) -> LnurlPayClient,
    internal val authenticator: SendAuthenticator,
    internal val ensureLspConnected: () -> Unit,
    internal val io: CoroutineDispatcher = Dispatchers.IO,
    internal val nowEpochSecs: () -> Long = ::epochSecondsNow,
    internal val nanoTime: () -> Long = System::nanoTime,
) : ViewModel() {
    internal val mutableState = MutableStateFlow(SendFlowState())
    val state: StateFlow<SendFlowState> = mutableState.asStateFlow()

    private var recipientJob: Job? = null
    internal var lnurlQuoteJob: Job? = null
    internal var pendingSettlementJob: Job? = null

    internal fun update(transform: (SendFlowState) -> SendFlowState) =
        mutableState.update(transform)

    /** The LNURL client for the node's network, or null before the node starts. */
    internal fun lnurlClient(): LnurlPayClient? = wallet.network?.let(lnurlClientFor)

    /** Moves to [step]. A quote belongs to one visit of the confirm step. */
    fun goTo(step: SendFlowStep) {
        if (state.value.step == SendFlowStep.CONFIRM && step != SendFlowStep.CONFIRM) {
            invalidateLnurlQuote()
        }
        update { it.copy(step = step) }
    }

    fun setInput(text: String) {
        val current = state.value
        if (text == current.inputText) return
        val classification = classifier.classify(text, wallet.network)
        val target = (classification as? DestinationClassification.Valid)?.destination
        update { s ->
            val base = s.copy(inputText = text, classification = classification, error = null)
            when {
                target == null ->
                    base.copy(
                        destination = null,
                        isMaxAmount = false,
                        lnurlParams = null,
                        lnurlComment = "",
                        amountInputText = "",
                    )
                target == s.destination -> base
                else -> {
                    val presetSats = target.presetSats
                    base.copy(
                        destination = target,
                        isMaxAmount = false,
                        lnurlParams = null,
                        lnurlComment = "",
                        amountUnit = if (presetSats != null) SendAmountUnit.SATS else s.amountUnit,
                        amountInputText = presetSats?.toString() ?: "",
                    )
                }
            }
        }
    }

    /** Starts recipient resolution unless one is already in flight. */
    fun startProceedFromRecipient() {
        if (recipientJob != null) return
        // Started after the handle is stored, so a fast finish cannot clear it before it is set.
        val job =
            viewModelScope.launch(start = CoroutineStart.LAZY) {
                try {
                    proceedFromRecipient()
                } finally {
                    if (recipientJob === coroutineContext.job) recipientJob = null
                }
            }
        recipientJob = job
        job.start()
    }

    /** Abandons an in-flight Lightning Address or LNURL fetch and returns to the recipient step. */
    fun cancelProceedFromRecipient() {
        recipientJob?.cancel()
        recipientJob = null
        update { it.copy(isFetchingLnurl = false, error = null) }
        goTo(SendFlowStep.RECIPIENT)
    }

    internal suspend fun proceedFromRecipient() {
        val destination = state.value.destination ?: return
        update { it.copy(error = null) }
        when (destination) {
            is SendDestination.LightningAddress,
            is SendDestination.LnurlPay -> fetchPayParams(destination)
            is SendDestination.Bolt11 -> checkBolt11(destination)
            is SendDestination.Bolt12 -> goTo(SendFlowStep.AMOUNT)
            is SendDestination.Onchain -> {
                // A BIP-21 amount refills the field if the user cleared it.
                val preset = destination.presetSats
                if (preset != null && state.value.amountInputText.isEmpty()) {
                    update {
                        it.copy(amountUnit = SendAmountUnit.SATS, amountInputText = "$preset")
                    }
                }
                goTo(SendFlowStep.AMOUNT)
            }
        }
    }

    private suspend fun fetchPayParams(destination: SendDestination) {
        val client = lnurlClient() ?: return fail(SendError.WalletNotReady)
        val url =
            when (destination) {
                is SendDestination.LightningAddress -> destination.url
                is SendDestination.LnurlPay -> destination.url
                else -> return
            }
        update { it.copy(isFetchingLnurl = true) }
        try {
            val params = withContext(io) { client.fetchPayParams(url) }
            update { it.copy(lnurlParams = params, isFetchingLnurl = false) }
            goTo(SendFlowStep.AMOUNT)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            update { it.copy(isFetchingLnurl = false) }
            fail(e.toSendError())
        }
    }

    private fun checkBolt11(destination: SendDestination.Bolt11) {
        val decoded = decoder.decodeBolt11(destination.raw)
        val payability =
            InvoicePayability.evaluate(
                isExpired = decoded?.isExpired(nowEpochSecs()) == true,
                existingPayment = decoded?.paymentHash?.let(wallet::outboundPaymentStatus),
            )
        if (payability != InvoicePayability.PAYABLE) {
            return fail(SendError.InvoiceNotPayable(payability))
        }
        if (destination.amountMsat?.takeIf { it > 0 } == null) return goTo(SendFlowStep.AMOUNT)
        val error = SendAmountValidator.validate(amountRequest() ?: return, quote() ?: return)
        if (error != null) return fail(error)
        goTo(SendFlowStep.CONFIRM)
    }

    fun proceedFromAmount() {
        normalizeAmountInput()
        val s = state.value
        val request = amountRequest(s) ?: return
        val error = SendAmountValidator.validate(request, quote(s) ?: return)
        if (error != null) return fail(error)
        update { it.copy(error = null) }
        goTo(SendFlowStep.CONFIRM)
        loadLnurlQuoteIfNeeded()
    }

    /** The amount request for the current state, or null without a destination. */
    internal fun amountRequest(s: SendFlowState = state.value): SendAmountRequest? {
        val destination = s.destination ?: return null
        return SendAmountRequest(
            destination = destination,
            enteredSats =
                SendAmountCalculator.effectiveSats(
                    destination,
                    s.amountInputText,
                    s.amountUnit,
                    wallet.accountingPrice(),
                ),
            feeRateSatVb = s.estimateFeeRateSatVb,
            isMax = s.isMaxAmount,
            sendsAllFromOnchainWallet = s.sendsAllFromOnchainWallet,
            payeeLimits = s.lnurlParams?.let { PayeeAmountLimits(it.minSats, it.maxSats) },
            quotedAmountSats = s.lnurlQuote?.let { it.amountMsat / 1000 },
        )
    }

    /** What this send moves, costs and can spend, priced against the wallet now. */
    fun quote(s: SendFlowState = state.value): SendQuote? =
        amountRequest(s)?.let { SendAmountValidator.quote(it, wallet.balances()) }

    fun setLnurlComment(comment: String) = update { it.copy(lnurlComment = comment) }

    /** Settings "Send All" to [address]: the whole on-chain balance, never a splice-out. */
    fun startSendAllFromOnchainWallet(address: String) {
        setInput(address)
        update { it.copy(sendsAllFromOnchainWallet = true) }
    }

    internal fun fail(error: SendError) = update { it.copy(error = error) }

    fun resetFlow() {
        recipientJob?.cancel()
        recipientJob = null
        pendingSettlementJob?.cancel()
        pendingSettlementJob = null
        invalidateLnurlQuote()
        // Fee rates and the chosen tier outlive one send, as on iOS.
        update {
            SendFlowState(
                recommendedFees = it.recommendedFees,
                selectedFeeTier = it.selectedFeeTier,
            )
        }
    }
}

/** Typed LNURL errors keep their reason; anything else is a node or transport failure. */
internal fun Throwable.toSendError(): SendError =
    when (this) {
        is SendFailure -> error
        is LnurlError -> SendError.Lnurl(this)
        else -> SendError.Dispatch(this)
    }
