package com.stablechannels.app.services

import com.stablechannels.app.util.Constants
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.security.SecureRandom

data class StabilityPayment(
    val settlementId: String,
    val channelId: String,
    val amountMsat: Long,
    val direction: String,
    val expectedUsd: Double,
    val createdAt: Long,
    val expiresAt: Long
)

sealed interface SignedSettlementValidation {
    data class Valid(val payment: StabilityPayment) : SignedSettlementValidation
    data class Invalid(val reason: String) : SignedSettlementValidation
}

/** STABILITY_PAYMENT_V1 payload/envelope codec shared by both send paths and both receive
 *  paths. Mirrors stable.rs build/parse_stability_payment_payload exactly so the wire format
 *  stays interchangeable with the Rust peers. */
object StabilityPaymentProtocol {
    const val DIRECTION_USER_TO_LSP = "user_to_lsp"
    const val DIRECTION_LSP_TO_USER = "lsp_to_user"

    fun newSettlementId(): String = ByteArray(32)
        .also { SecureRandom().nextBytes(it) }
        .joinToString("") { "%02x".format(it) }

    fun buildPayload(
        settlementId: String,
        channelId: String,
        amountMsat: Long,
        direction: String,
        expectedUsd: Double,
        createdAt: Long,
        expiresAt: Long
    ): String? {
        if (!TradeProtocol.isCanonicalIdentifier(settlementId) ||
            !TradeProtocol.isCanonicalIdentifier(channelId) ||
            amountMsat <= 0L || amountMsat % 1000L != 0L ||
            (direction != DIRECTION_USER_TO_LSP && direction != DIRECTION_LSP_TO_USER) ||
            !expectedUsd.isFinite() || expectedUsd < 0.0 ||
            createdAt < 0L || expiresAt < createdAt ||
            expiresAt - createdAt > Constants.STABILITY_PAYMENT_TTL_SECS
        ) return null
        return JSONObject().apply {
            put("type", Constants.STABILITY_PAYMENT_MESSAGE_TYPE)
            put("settlement_id", settlementId)
            put("channel_id", channelId)
            put("amount_msat", amountMsat)
            put("direction", direction)
            put("expected_usd", expectedUsd)
            put("created_at", createdAt)
            put("expires_at", expiresAt)
        }.toString()
    }

    /** Signed envelope for an outgoing user_to_lsp settlement, or null when the inputs cannot
     *  produce a payload the LSP would accept. The caller must send the same whole-sat amount. */
    fun buildSignedEnvelope(
        channelId: String,
        amountMsat: Long,
        expectedUsd: Double,
        sign: (ByteArray) -> String,
        now: Long = System.currentTimeMillis() / 1000L,
        settlementId: String = newSettlementId()
    ): String? {
        val payload = buildPayload(
            settlementId, channelId, amountMsat, DIRECTION_USER_TO_LSP,
            expectedUsd, now, now + Constants.STABILITY_PAYMENT_TTL_SECS
        ) ?: return null
        val signature = sign(payload.toByteArray(Charsets.UTF_8))
        return JSONObject().apply {
            put("payload", payload)
            put("signature", signature)
        }.toString()
    }

    fun parsePayload(payloadStr: String): StabilityPayment? {
        return try {
            val payload = JSONObject(payloadStr)
            if (payload.optString("type") != Constants.STABILITY_PAYMENT_MESSAGE_TYPE) return null
            val settlementId = payload.optString("settlement_id")
            val channelId = payload.optString("channel_id")
            val amountMsat = jsonInteger(payload, "amount_msat")
            val direction = payload.optString("direction")
            val expectedUsd = (payload.opt("expected_usd") as? Number)?.toDouble()
            val createdAt = jsonInteger(payload, "created_at")
            val expiresAt = jsonInteger(payload, "expires_at")
            if (!TradeProtocol.isCanonicalIdentifier(settlementId) ||
                !TradeProtocol.isCanonicalIdentifier(channelId) ||
                amountMsat == null || amountMsat <= 0L || amountMsat % 1000L != 0L ||
                (direction != DIRECTION_USER_TO_LSP && direction != DIRECTION_LSP_TO_USER) ||
                expectedUsd == null || !expectedUsd.isFinite() || expectedUsd < 0.0 ||
                createdAt == null || createdAt < 0L ||
                expiresAt == null || expiresAt < createdAt ||
                expiresAt - createdAt > Constants.STABILITY_PAYMENT_TTL_SECS
            ) return null
            StabilityPayment(
                settlementId, channelId, amountMsat, direction,
                expectedUsd, createdAt, expiresAt
            )
        } catch (_: Exception) {
            null
        }
    }

    fun isFresh(payment: StabilityPayment, now: Long): Boolean =
        payment.createdAt <= now + Constants.STABILITY_PAYMENT_CLOCK_SKEW_SECS &&
            now <= payment.expiresAt + Constants.STABILITY_PAYMENT_CLOCK_SKEW_SECS

    /** Validate an inbound SIGNED_STABILITY_TLV record against the received keysend.
     *  Check order mirrors user.rs handle_signed_stability_payment_received. */
    fun validateInbound(
        data: ByteArray,
        expectedCounterparty: String,
        ownChannelId: String,
        amountMsat: Long,
        now: Long = System.currentTimeMillis() / 1000L,
        verifySignature: (ByteArray, String, String) -> Boolean
    ): SignedSettlementValidation {
        if (data.size > Constants.MAX_SIGNED_STABILITY_TLV_VALUE_BYTES) {
            return SignedSettlementValidation.Invalid("oversize")
        }
        val raw = try {
            Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(data)).toString()
        } catch (_: Exception) {
            return SignedSettlementValidation.Invalid("utf8")
        }
        val envelope = try { JSONObject(raw) } catch (_: Exception) {
            return SignedSettlementValidation.Invalid("envelope")
        }
        val payloadStr = envelope.opt("payload") as? String
            ?: return SignedSettlementValidation.Invalid("envelope")
        val signature = envelope.opt("signature") as? String
            ?: return SignedSettlementValidation.Invalid("envelope")
        val payment = parsePayload(payloadStr)
            ?: return SignedSettlementValidation.Invalid("fields")
        if (payment.direction != DIRECTION_LSP_TO_USER) {
            return SignedSettlementValidation.Invalid("direction")
        }
        if (payment.amountMsat != amountMsat) {
            return SignedSettlementValidation.Invalid("amount")
        }
        if (!isFresh(payment, now)) {
            return SignedSettlementValidation.Invalid("expired")
        }
        if (payment.channelId != ownChannelId) {
            return SignedSettlementValidation.Invalid("channel")
        }
        if (!verifySignature(payloadStr.toByteArray(Charsets.UTF_8), signature, expectedCounterparty)) {
            return SignedSettlementValidation.Invalid("signature")
        }
        return SignedSettlementValidation.Valid(payment)
    }

    private fun jsonInteger(payload: JSONObject, key: String): Long? {
        val value = payload.opt(key)
        return when (value) {
            is Byte -> value.toLong()
            is Short -> value.toLong()
            is Int -> value.toLong()
            is Long -> value
            else -> null
        }
    }
}
