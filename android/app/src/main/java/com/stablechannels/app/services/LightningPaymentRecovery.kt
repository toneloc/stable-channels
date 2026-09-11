package com.stablechannels.app.services

/** Terminal LDK state needed to repair an outbound Lightning history row. */
data class LightningPaymentResolution(val succeeded: Boolean, val feeMsat: Long = 0L)

/** Shared persistence path for foreground recovery and background event consumers. */
object LightningPaymentRecovery {
    fun recordSuccess(db: DatabaseService, paymentId: String?, feeMsat: Long?): Boolean {
        if (paymentId.isNullOrEmpty()) return false
        // Trade fees are tracked in the trades table as well as the payment history.
        db.markTradeFeePaid(paymentId)
        db.updatePaymentStatus(paymentId, "completed", feeMsat ?: 0L)
        return true
    }

    fun recordFailure(db: DatabaseService, paymentId: String?, reasonCode: String?): Boolean {
        if (paymentId.isNullOrEmpty()) return false
        PaymentFailureRecorder.record(db, paymentId, reasonCode) { null }
        return true
    }

    /** Reconcile rows whose event may have been consumed while Android was backgrounded. */
    fun reconcilePending(
        db: DatabaseService,
        lookup: (String) -> LightningPaymentResolution?
    ): Int {
        var repaired = 0
        db.getPendingOutgoingLightningPaymentIds().forEach { paymentId ->
            when (val resolution = lookup(paymentId)) {
                is LightningPaymentResolution -> {
                    if (resolution.succeeded) {
                        db.updatePaymentStatus(paymentId, "completed", resolution.feeMsat)
                    } else {
                        PaymentFailureRecorder.record(db, paymentId, null) { null }
                    }
                    repaired++
                }
                null -> Unit
            }
        }
        return repaired
    }
}
