package com.stablechannels.app.services

/** Terminal Lightning result. AppState keys these by payment ID, never by display text. */
data class PaymentOutcome(
    val succeeded: Boolean,
    val message: String,
    val observedAtNanos: Long = System.nanoTime()
) {
    // A BOLT11 invoice can reuse its payment ID on retry. Ignore a previous attempt's result.
    fun belongsToAttempt(startedAtNanos: Long): Boolean = observedAtNanos - startedAtNanos >= 0
}
