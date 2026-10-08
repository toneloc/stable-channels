package com.stablechannels.app.ui.components

import com.stablechannels.app.services.PaymentOutcome
import com.stablechannels.app.services.TradeOutcome

object PaymentResultControls {
    fun showsTradeDone(outcome: TradeOutcome?): Boolean = outcome != null

    fun isLightningPending(
        paymentId: String?,
        outcome: PaymentOutcome?,
        attemptStartedAtNanos: Long,
    ): Boolean = paymentId != null && outcome?.belongsToAttempt(attemptStartedAtNanos) != true
}
