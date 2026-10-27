package com.stablechannels.app.services.send

import com.stablechannels.app.domain.send.PaymentSettlement
import com.stablechannels.app.domain.send.PaymentSettlementWatcher
import com.stablechannels.app.services.PaymentOutcome
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Waits for a Lightning payment to settle or fail. [outcomes] keeps every result by payment id, so
 * a payment that settles before anyone waits is still found: only results observed after the
 * attempt started count, and the caller takes that time before dispatching.
 */
class PaymentSettlementObserver(
    private val outcomes: StateFlow<Map<String, PaymentOutcome>>,
    /** Asks the node for the status, in case its event was handled elsewhere. */
    private val refresh: (paymentId: String, attemptStartedAtNanos: Long) -> Unit,
    private val pollIntervalMs: Long = 2_000,
) : PaymentSettlementWatcher {
    override suspend fun await(
        paymentId: String,
        attemptStartedAtNanos: Long,
        timeoutMs: Long,
    ): PaymentSettlement =
        withTimeoutOrNull(timeoutMs) {
            coroutineScope {
                val poller = launch {
                    while (true) {
                        refresh(paymentId, attemptStartedAtNanos)
                        delay(pollIntervalMs)
                    }
                }
                val outcome =
                    outcomes
                        .mapNotNull {
                            it[paymentId]?.takeIf { o -> o.belongsToAttempt(attemptStartedAtNanos) }
                        }
                        .first()
                poller.cancel()
                if (outcome.succeeded) PaymentSettlement.Settled
                else PaymentSettlement.Failed(outcome.message)
            }
        } ?: PaymentSettlement.TimedOut
}
