package com.stablechannels.app.services

data class RecordedPaymentFailure(
    val isTrade: Boolean = false,
    val tradeOutcome: TradeOutcome? = null,
    val isStability: Boolean = false
)

/** Commit before acknowledging the LDK event in either Android event consumer. */
object PaymentFailureRecorder {
    fun record(
        db: DatabaseService,
        paymentId: String,
        reasonCode: String?,
        lookupAmountMsat: () -> Long?
    ): RecordedPaymentFailure {
        val pendingSend = db.loadPendingSend()
        if (pendingSend?.paymentId == paymentId) {
            db.clearPendingSend()
            return RecordedPaymentFailure(isStability = true)
        }
        if (db.tradePaymentExists(paymentId)) {
            // Signed results and already-paid fees are protected from late failure events.
            db.markTradePaymentFailed(paymentId, reasonCode)
            return RecordedPaymentFailure(isTrade = true, tradeOutcome = db.terminalTradeOutcome(paymentId))
        }
        if (db.hasUnattachedPreparedTrade()) {
            val amount = lookupAmountMsat()
            if (amount != null && db.failUnattachedPreparedTrade(paymentId, amount, reasonCode) != null) {
                return RecordedPaymentFailure(isTrade = true, tradeOutcome = db.terminalTradeOutcome(paymentId))
            }
        }
        db.updatePaymentStatus(paymentId, "failed")
        return RecordedPaymentFailure()
    }
}
