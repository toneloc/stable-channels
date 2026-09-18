package com.stablechannels.app.services

import java.security.MessageDigest
import java.security.SecureRandom
import org.lightningdevkit.ldknode.NodeException

/** A stability keysend whose payment id is on the claim before LDK is asked to send it. */
object StabilityKeysend {
    sealed class Outcome {
        data class Sent(val paymentId: String) : Outcome()

        /** Nothing left the node; the caller releases the claim. */
        data class NotSent(val error: Exception) : Outcome()

        /** LDK dispatched before failing to save its record; the claim and its id are kept. */
        data class OutcomeUnknown(val error: Exception) : Outcome()
    }

    fun newPreimage(): String = ByteArray(32).also { SecureRandom().nextBytes(it) }.toHex()

    /** LDK derives a spontaneous payment's id from its preimage, so the preimage fixes the id. */
    fun paymentId(preimageHex: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(preimageHex.chunked(2).map { it.toInt(16).toByte() }.toByteArray())
            .toHex()

    fun send(db: DatabaseService, send: (preimageHex: String) -> String): Outcome {
        val preimage = newPreimage()
        val claimedId = paymentId(preimage)
        try {
            db.setPendingSendPaymentId(claimedId)
        } catch (e: Exception) {
            return Outcome.NotSent(e)
        }
        val ldkId =
            try {
                send(preimage)
            } catch (e: NodeException.PersistenceFailed) {
                AuditService.log(
                    "STABILITY_PAYMENT_OUTCOME_UNKNOWN",
                    mapOf("payment_id" to claimedId, "error" to (e.message ?: "")),
                )
                return Outcome.OutcomeUnknown(e)
            } catch (e: Exception) {
                return Outcome.NotSent(e)
            }
        if (ldkId != claimedId) {
            // Events carry LDK's id, so the claim must too.
            AuditService.log(
                "STABILITY_PAYMENT_ID_MISMATCH",
                mapOf("claimed_id" to claimedId, "ldk_id" to ldkId),
            )
            try {
                db.setPendingSendPaymentId(ldkId)
            } catch (_: Exception) {}
        }
        return Outcome.Sent(ldkId)
    }

    private fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }
}
