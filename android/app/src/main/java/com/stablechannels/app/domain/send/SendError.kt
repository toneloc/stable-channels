package com.stablechannels.app.domain.send

import com.stablechannels.app.domain.lnurl.LnurlError

/** A send rejected for a reason the flow shows as is. */
class SendFailure(val error: SendError) : Exception()

/** Why a send cannot go ahead. The UI maps each case to localized text. */
sealed class SendError {
    data object Offline : SendError()

    data object FeeRateNotReady : SendError()

    data object InvalidAmount : SendError()

    data class OutsidePayeeLimits(val minSats: Long, val maxSats: Long) : SendError()

    /** The amount alone is more than the wallet can spend. */
    data class AmountExceedsBalance(val availableSats: Long) : SendError()

    /** The amount fits but amount plus fee does not, or nothing is spendable. */
    data class InsufficientBalance(val availableSats: Long) : SendError()

    /** The wallet network is unknown until the node starts, so LNURL cannot be resolved yet. */
    data object WalletNotReady : SendError()

    data class InvoiceNotPayable(val reason: InvoicePayability) : SendError()

    data object MissingLnurlInvoice : SendError()

    /** The quoted invoice expired; a fresh one is loading and must be reviewed again. */
    data object LnurlInvoiceRefreshed : SendError()

    data class Lnurl(val error: LnurlError) : SendError()

    /** A splice is already moving the on-chain funds. */
    data object SpliceInProgress : SendError()

    /** No ready channel can fund the splice-out. */
    data object NoSpliceChannel : SendError()

    data object AuthenticationRequired : SendError()

    /** The payment was dispatched and then failed; [reason] is the node text, if any. */
    data class PaymentFailed(val reason: String?) : SendError()

    /** The node rejected the send. */
    data class Dispatch(val cause: Throwable) : SendError()
}
