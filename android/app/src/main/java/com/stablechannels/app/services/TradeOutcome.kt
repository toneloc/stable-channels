package com.stablechannels.app.services

/** A fee send failure is terminal, but is not a signed rejection by the provider. */
data class TradeOutcome(val accepted: Boolean, val message: String, val sendFailed: Boolean = false) {
    companion object {
        fun fromStored(status: String, reasonCode: String?): TradeOutcome? = when (status) {
            "accepted" -> TradeOutcome(true, "")
            "rejected" -> TradeOutcome(false, TradeProtocol.rejectionMessage(reasonCode ?: "internal_failure"))
            "send_failed" -> TradeOutcome(false,
                "The trade fee payment failed. " + WalletErrorMessages.paymentFailureCode(reasonCode),
                sendFailed = true)
            else -> null
        }
    }
}
