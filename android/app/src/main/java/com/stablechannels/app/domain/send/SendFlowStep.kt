package com.stablechannels.app.domain.send

/** Active step of the send flow. */
enum class SendFlowStep {
    RECIPIENT,
    AMOUNT,
    CONFIRM,
    SUCCESS,
}
