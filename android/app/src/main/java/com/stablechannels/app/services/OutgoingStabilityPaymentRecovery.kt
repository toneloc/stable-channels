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
            val candidates = node.listPayments().filter { matchesUnassigned(pending, it) }
            // Amount/time matching is a legacy crash-recovery fallback, not an identity proof.
            // Never choose arbitrarily among multiple possible sends.
            if (candidates.size > 1) return false
            val candidate = candidates.singleOrNull()
            if (candidate == null) {
                return if (System.currentTimeMillis() / 1000 - pending.createdAt > 120) {
                    db.clearPendingSend(pending)
                } else false
            }
            if (!db.adoptPendingSendPaymentId(pending, candidate.id)) return false
            pending = pending.copy(paymentId = candidate.id)
        }

        val payment = node.payment(pending.paymentId) ?: return false
        if (payment.kind !is PaymentKind.Spontaneous || payment.direction != PaymentDirection.OUTBOUND ||
            payment.amountMsat?.toLong() != pending.amountMsat) return false
        when (payment.status) {
            PaymentStatus.PENDING -> return false
            PaymentStatus.FAILED -> return db.clearPendingSend(pending)
            PaymentStatus.SUCCEEDED -> Unit
        }
        // An old marker may outlive its atomic history/backing commit. Clear only that proven
        // accounting; otherwise preserve the unknown-origin obligation without guessing a channel.
        val origin = pending.userChannelId?.takeIf { it.isNotBlank() }
            ?: return db.clearAccountedLegacyStabilitySend(pending)
        val closed = channelsAuthoritative && node.listChannels().none { it.userChannelId == origin }
        return db.completePendingStabilitySend(pending, channelClosed = closed)
    }

    internal fun matchesUnassigned(pending: PendingStabilitySend, payment: PaymentDetails): Boolean =
        payment.direction == PaymentDirection.OUTBOUND && payment.kind is PaymentKind.Spontaneous &&
            payment.amountMsat?.toLong() == pending.amountMsat &&
            payment.latestUpdateTimestamp.toLong() >= pending.createdAt - 10
}
