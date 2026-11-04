package com.stablechannels.app.ui.transfer.send

import com.stablechannels.app.domain.lnurl.LnurlInvoiceQuote
import com.stablechannels.app.domain.lnurl.LnurlPayParams
import com.stablechannels.app.domain.send.DestinationClassification
import com.stablechannels.app.domain.send.NetworkFeeSpeedTier
import com.stablechannels.app.domain.send.RecommendedFees
import com.stablechannels.app.domain.send.SendAmountUnit
import com.stablechannels.app.domain.send.SendDestination
import com.stablechannels.app.domain.send.SendError
import com.stablechannels.app.domain.send.SendFlowStep
import com.stablechannels.app.domain.send.SendSuccessStatus
import com.stablechannels.app.util.Constants

/** What the receipt shows after a send was dispatched. */
data class SendReceipt(
    val sentAmountSats: Long,
    val paymentId: String? = null,
    val txid: String? = null,
    val isPendingSettlement: Boolean,
) {
    val status: SendSuccessStatus
        get() = SendSuccessStatus.of(isPendingSettlement, txid)
}

/** Everything the send screens render. */
data class SendFlowState(
    val step: SendFlowStep = SendFlowStep.RECIPIENT,
    val inputText: String = "",
    val classification: DestinationClassification = DestinationClassification.Empty,
    val destination: SendDestination? = null,
    val amountUnit: SendAmountUnit = SendAmountUnit.USD,
    val amountInputText: String = "",
    /** The user chose Max: the amount follows the balance and the selected fee. */
    val isMaxAmount: Boolean = false,
    /**
     * Settings "Send All": empties the on-chain wallet in one transaction even when a channel is
     * ready, so unlike Max it is never a splice-out.
     */
    val sendsAllFromOnchainWallet: Boolean = false,
    val lnurlParams: LnurlPayParams? = null,
    val lnurlComment: String = "",
    val isFetchingLnurl: Boolean = false,
    /** The invoice the confirm step shows; the slide pays exactly this invoice. */
    val lnurlQuote: LnurlInvoiceQuote? = null,
    val isFetchingLnurlQuote: Boolean = false,
    val lnurlQuoteError: SendError? = null,
    val recommendedFees: RecommendedFees? = null,
    val selectedFeeTier: NetworkFeeSpeedTier = NetworkFeeSpeedTier.STANDARD,
    val isSending: Boolean = false,
    val error: SendError? = null,
    val receipt: SendReceipt? = null,
    /** Bumped when a send attempt ends, so the slider resets. */
    val resetToken: Int = 0,
) {
    val isLnurl: Boolean
        get() =
            destination is SendDestination.LightningAddress ||
                destination is SendDestination.LnurlPay

    /** The live rate of the selected tier, or null until fees load. */
    val effectiveFeeRateSatVb: Double?
        get() = recommendedFees?.rate(selectedFeeTier)

    /** The rate estimates use: the live rate, or a placeholder until it loads. */
    val estimateFeeRateSatVb: Double
        get() =
            effectiveFeeRateSatVb
                ?: selectedFeeTier.effectiveRate(Constants.PLACEHOLDER_FEE_RATE_SAT_VB)

    /** An on-chain send waits for a real fee rate; Lightning does not need one. */
    val isFeeRateReady: Boolean
        get() = destination !is SendDestination.Onchain || recommendedFees != null

    /** False while an LNURL send has no invoice to show, so the slide stays disabled. */
    val isLnurlQuoteReady: Boolean
        get() = !isLnurl || lnurlQuote != null
}
