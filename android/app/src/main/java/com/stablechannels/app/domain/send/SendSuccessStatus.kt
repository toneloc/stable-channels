package com.stablechannels.app.domain.send

/** How final a dispatched payment is, as the receipt shows it. */
enum class SendSuccessStatus {
    /** Lightning payment settled. */
    COMPLETED,

    /** On-chain transaction broadcast but not yet confirmed. */
    AWAITING_CONFIRMATION,

    /** Lightning settlement or splice-out still in flight. */
    PENDING;

    companion object {
        /** Unsettled sends stay pending; a broadcast transaction awaits confirmation. */
        fun of(isPendingSettlement: Boolean, txid: String?): SendSuccessStatus =
            when {
                isPendingSettlement -> PENDING
                txid != null -> AWAITING_CONFIRMATION
                else -> COMPLETED
            }
    }
}
