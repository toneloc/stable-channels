package com.stablechannels.app.domain.send

/**
 * How a dispatched payment settles. A splice-out has neither a payment id to await nor a txid until
 * negotiation completes, so it is its own case rather than a pair of nulls.
 */
sealed class SendPaymentOutcome {
    /** Lightning payment in flight; await settlement by payment id. */
    data class Lightning(val paymentId: String) : SendPaymentOutcome()

    /** On-chain transaction already broadcast. */
    data class Onchain(val txid: String) : SendPaymentOutcome()

    /** Splice-out started from the channel; it confirms on-chain and Home tracks it. */
    data object SpliceOut : SendPaymentOutcome()
}

data class SendPaymentResult(val sentAmountSats: Long, val outcome: SendPaymentOutcome)

/** What waiting for a Lightning payment found. */
sealed class PaymentSettlement {
    data object Settled : PaymentSettlement()

    /** [reason] is the node's failure text, or null when it gave none. */
    data class Failed(val reason: String?) : PaymentSettlement()

    /** Not settled in time. Never shown as sent: the receipt stays pending. */
    data object TimedOut : PaymentSettlement()
}

/** The Lightning payment types a send records in history. */
enum class LightningPaymentKind {
    BOLT11,
    BOLT12,
    LNURL,
}

/** How long a send waits for a Lightning settlement before showing the receipt as pending. */
object SettlementWait {
    const val BOLT11_MS = 7_000L

    /** Longer: the node first fetches an invoice from the offer. */
    const val BOLT12_MS = 10_000L

    /** A pending receipt is checked again for this long per round. */
    const val PENDING_ROUND_MS = 60_000L

    fun forDestination(destination: SendDestination): Long =
        if (destination is SendDestination.Bolt12) BOLT12_MS else BOLT11_MS
}
