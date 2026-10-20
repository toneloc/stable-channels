package com.stablechannels.app.domain.send

/** Status of an earlier outbound payment for the same payment hash. */
enum class OutboundPaymentStatus {
    PENDING,
    SUCCEEDED,
    FAILED,
}

/** Whether a BOLT11 invoice can still be paid by this wallet. */
enum class InvoicePayability {
    PAYABLE,
    EXPIRED,
    ALREADY_PAID,
    PAYMENT_IN_PROGRESS;

    companion object {
        /**
         * An expired invoice is never payable. Otherwise an earlier payment for the same hash
         * decides: succeeded means paid, pending means in flight, failed allows a retry.
         */
        fun evaluate(
            isExpired: Boolean,
            existingPayment: OutboundPaymentStatus?,
        ): InvoicePayability =
            when {
                isExpired -> EXPIRED
                existingPayment == OutboundPaymentStatus.SUCCEEDED -> ALREADY_PAID
                existingPayment == OutboundPaymentStatus.PENDING -> PAYMENT_IN_PROGRESS
                else -> PAYABLE
            }
    }
}
