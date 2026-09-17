package com.stablechannels.app.services

import org.lightningdevkit.ldknode.Node
import org.lightningdevkit.ldknode.PaymentDetails
import org.lightningdevkit.ldknode.PaymentDirection
import org.lightningdevkit.ldknode.PaymentKind
import org.lightningdevkit.ldknode.PaymentStatus

/** The same deferred-settlement policy runs in the foreground and background processes. */
object OutgoingStabilityPaymentRecovery {
    fun reconcile(db: DatabaseService, node: Node, channelsAuthoritative: Boolean): Boolean {
        var pending = db.loadPendingSend() ?: return true
        if (pending.paymentId.isEmpty()) {
            // A trade fee keysend is never the stability payment, whatever its amount.
            val candidates =
                node.listPayments().filter {
                    matchesUnassigned(pending, it) && !db.tradePaymentExists(it.id)
                }
            // Amount/time matching is a legacy crash-recovery fallback, not an identity proof.
            // Every candidate carries the claimed amount, so one shared terminal outcome gives
            // the same accounting whichever was ours and the earliest is adopted. Conflicting
            // outcomes release the barrier without a debit: either guess could be wrong.
            val candidate =
                candidates.singleOrNull()
                    ?: when {
                        candidates.isEmpty() -> null
                        candidates.any { it.status == PaymentStatus.PENDING } -> return false
                        candidates.all { it.status == candidates.first().status } ->
                            candidates.minBy { it.latestUpdateTimestamp }
                        else ->
                            return db.clearPendingSend(pending).also { released ->
                                if (released)
                                    AuditService.log(
                                        "STABILITY_MARKER_RELEASED_AMBIGUOUS",
                                        mapOf(
                                            "amount_msat" to pending.amountMsat,
                                            "candidates" to candidates.joinToString(",") { it.id },
                                        ),
                                    )
                            }
                    }
            if (candidate == null) {
                return if (System.currentTimeMillis() / 1000 - pending.createdAt > 120) {
                    db.clearPendingSend(pending)
                } else false
            }
            if (!db.adoptPendingSendPaymentId(pending, candidate.id)) return false
            pending = pending.copy(paymentId = candidate.id)
        }

        val payment =
            node.payment(pending.paymentId)
                ?: return releaseLostRecord(db, pending, channelsAuthoritative)
        if (
            payment.kind !is PaymentKind.Spontaneous ||
                payment.direction != PaymentDirection.OUTBOUND ||
                payment.amountMsat?.toLong() != pending.amountMsat
        )
            return false
        when (payment.status) {
            PaymentStatus.PENDING -> return false
            PaymentStatus.FAILED -> return db.clearPendingSend(pending)
            PaymentStatus.SUCCEEDED -> Unit
        }
        // An old marker may outlive its atomic history/backing commit, or precede it entirely.
        // Clear proven accounting, debit only a proven origin, and otherwise keep the payment on
        // record while releasing the barrier: never guess a channel.
        val origin =
            pending.userChannelId?.takeIf { it.isNotBlank() }
                ?: run {
                    if (db.clearAccountedLegacyStabilitySend(pending)) return true
                    val adopted =
                        db.adoptLegacyStabilityOrigin(pending)
                            ?: return db.recordUnattributedLegacyStabilitySend(pending).also {
                                released ->
                                if (released)
                                    AuditService.log(
                                        "STABILITY_LEGACY_UNATTRIBUTED",
                                        mapOf(
                                            "payment_id" to pending.paymentId,
                                            "amount_msat" to pending.amountMsat,
                                            "price" to pending.price,
                                        ),
                                    )
                            }
                    pending = pending.copy(userChannelId = adopted)
                    adopted
                }
        val closed =
            channelsAuthoritative && node.listChannels().none { it.userChannelId == origin }
        return db.completePendingStabilitySend(pending, channelClosed = closed)
    }

    // Only a running node's store can prove a record is missing. After the grace period the marker
    // stops blocking spends; its origin stays on record, so a late success still debits it once.
    private fun releaseLostRecord(
        db: DatabaseService,
        pending: PendingStabilitySend,
        storeAuthoritative: Boolean,
    ): Boolean {
        val ageSecs = System.currentTimeMillis() / 1000 - pending.createdAt
        if (!storeAuthoritative || ageSecs <= LightningPaymentRecovery.LOST_LDK_RECORD_TIMEOUT_SECS)
            return false
        return db.releaseLostStabilitySend(pending).also { released ->
            if (released)
                AuditService.log(
                    "STABILITY_MARKER_RELEASED_NO_LDK_RECORD",
                    mapOf("payment_id" to pending.paymentId, "amount_msat" to pending.amountMsat),
                )
        }
    }

    internal fun matchesUnassigned(
        pending: PendingStabilitySend,
        payment: PaymentDetails,
    ): Boolean =
        payment.direction == PaymentDirection.OUTBOUND &&
            payment.kind is PaymentKind.Spontaneous &&
            payment.amountMsat?.toLong() == pending.amountMsat &&
            payment.latestUpdateTimestamp.toLong() >= pending.createdAt - 10
}
