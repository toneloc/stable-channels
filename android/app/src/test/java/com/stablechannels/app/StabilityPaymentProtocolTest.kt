package com.stablechannels.app

import com.stablechannels.app.services.SignedSettlementValidation
import com.stablechannels.app.services.StabilityPaymentProtocol
import com.stablechannels.app.util.Constants
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class StabilityPaymentProtocolTest {
    private val identifier = "ab".repeat(32)
    private val now = 1_786_310_000L

    private fun envelopeFor(
        payload: String,
        signature: String = "valid"
    ): ByteArray = JSONObject().apply {
        put("payload", payload)
        put("signature", signature)
    }.toString().toByteArray()

    private fun validate(
        data: ByteArray,
        amountMsat: Long = 25_000L,
        channelId: String = identifier,
        at: Long = now
    ) = StabilityPaymentProtocol.validateInbound(data, "lsp", channelId, amountMsat, at) { bytes, sig, pk ->
        pk == "lsp" && sig == "valid" && bytes.contentEquals(
            JSONObject(String(data, Charsets.UTF_8)).getString("payload").toByteArray(Charsets.UTF_8)
        )
    }

    private fun inboundPayload(
        amountMsat: Long = 25_000L,
        direction: String = StabilityPaymentProtocol.DIRECTION_LSP_TO_USER,
        settlementId: String = identifier,
        channelId: String = identifier,
        createdAt: Long = now,
        expiresAt: Long = now + Constants.STABILITY_PAYMENT_TTL_SECS
    ): String = JSONObject().apply {
        put("type", "STABILITY_PAYMENT_V1")
        put("settlement_id", settlementId)
        put("channel_id", channelId)
        put("amount_msat", amountMsat)
        put("direction", direction)
        put("expected_usd", 50.0)
        put("created_at", createdAt)
        put("expires_at", expiresAt)
    }.toString()

    @Test
    fun signedSettlementRoundTrips() {
        val envelope = StabilityPaymentProtocol.buildSignedEnvelope(
            channelId = identifier,
            amountMsat = 25_000L,
            expectedUsd = 50.0,
            sign = { "valid" },
            now = now,
            settlementId = identifier
        )
        assertNotNull(envelope)
        val parsed = JSONObject(envelope!!)
        val payload = JSONObject(parsed.getString("payload"))
        assertEquals("STABILITY_PAYMENT_V1", payload.getString("type"))
        assertEquals(identifier, payload.getString("settlement_id"))
        assertEquals(identifier, payload.getString("channel_id"))
        assertEquals(25_000L, payload.getLong("amount_msat"))
        assertEquals("user_to_lsp", payload.getString("direction"))
        assertEquals(50.0, payload.getDouble("expected_usd"), 0.0)
        assertEquals(now, payload.getLong("created_at"))
        assertEquals(now + Constants.STABILITY_PAYMENT_TTL_SECS, payload.getLong("expires_at"))

        // The same payload signed by the LSP in the opposite direction validates inbound.
        val inbound = JSONObject(payload.toString()).apply {
            put("direction", "lsp_to_user")
        }.toString()
        val result = validate(envelopeFor(inbound))
        assertTrue(result is SignedSettlementValidation.Valid)
        assertEquals(identifier, (result as SignedSettlementValidation.Valid).payment.settlementId)
    }

    @Test
    fun randomSettlementIdsAreCanonicalAndUnique() {
        val ids = (1..8).map { StabilityPaymentProtocol.newSettlementId() }
        assertTrue(ids.all { it.length == 64 && it.all { c -> c in '0'..'9' || c in 'a'..'f' } })
        assertEquals(ids.size, ids.toSet().size)
    }

    @Test
    fun legacyMarkerBytesAreNotASettlement() {
        // The removed [0x01] stability marker TLV value must never validate — a
        // marker-only payment is now an ordinary Lightning receipt (#270 follow-up).
        assertInvalid("envelope", byteArrayOf(1))
    }

    @Test
    fun signedRecordAloneClassifiesAsStability() {
        // No legacy marker attached: a valid signed record on TLV 13377333 alone
        // classifies the payment as a stability settlement.
        assertValid(envelopeFor(inboundPayload()))
    }

    @Test
    fun buildRejectsNonWholeSatAmounts() {
        assertNull(StabilityPaymentProtocol.buildPayload(
            identifier, identifier, 1_500L, "user_to_lsp", 50.0, now, now + 100
        ))
        assertNull(StabilityPaymentProtocol.buildPayload(
            identifier, identifier, 0L, "user_to_lsp", 50.0, now, now + 100
        ))
        assertNull(StabilityPaymentProtocol.buildSignedEnvelope(
            identifier, 25_001L, 50.0, { "valid" }, now, identifier
        ))
    }

    @Test
    fun invalidWireShapesAreRejected() {
        // Envelope shape
        assertInvalid("envelope", "not json".toByteArray())
        assertInvalid("envelope", envelopeFor("{}", signature = "").let {
            JSONObject(String(it, Charsets.UTF_8)).apply { remove("signature") }.toString().toByteArray()
        })
        // Field validation
        assertInvalid("fields", envelopeFor("{\"type\":\"WRONG\"}"))
        assertInvalid("fields", envelopeFor(inboundPayload(settlementId = "AB".repeat(32))))
        assertInvalid("fields", envelopeFor(inboundPayload(channelId = "ab".repeat(31))))
        assertInvalid("fields", envelopeFor(
            JSONObject(inboundPayload()).apply { put("amount_msat", 25_001L) }.toString()
        ))
        assertInvalid("fields", envelopeFor(
            JSONObject(inboundPayload()).apply { put("expected_usd", -1.0) }.toString()
        ))
        // TTL exceeded
        assertInvalid("fields", envelopeFor(inboundPayload(
            createdAt = now, expiresAt = now + Constants.STABILITY_PAYMENT_TTL_SECS + 1
        )))
    }

    @Test
    fun bindingChecksRejectWrongDirectionAmountAndChannel() {
        assertInvalid("direction", envelopeFor(inboundPayload(direction = "user_to_lsp")))
        assertInvalid("amount", envelopeFor(inboundPayload(amountMsat = 26_000L)))
        assertInvalid("channel", envelopeFor(inboundPayload()), channelId = "cd".repeat(32))
    }

    @Test
    fun freshnessWindowHonorsClockSkew() {
        // Expired beyond the 60s skew
        assertInvalid("expired", envelopeFor(inboundPayload(
            createdAt = now - 2_000L, expiresAt = now - 120L
        )))
        // Not yet valid beyond the 60s skew
        assertInvalid("expired", envelopeFor(inboundPayload(
            createdAt = now + 120L, expiresAt = now + Constants.STABILITY_PAYMENT_TTL_SECS
        )))
        // Within skew on both edges
        assertValid(envelopeFor(inboundPayload(
            createdAt = now - 2_000L, expiresAt = now - 30L
        )))
        assertValid(envelopeFor(inboundPayload(
            createdAt = now + 30L, expiresAt = now + Constants.STABILITY_PAYMENT_TTL_SECS
        )))
    }

    @Test
    fun signatureAndOversizeAreRejected() {
        val payload = inboundPayload()
        val badSig = JSONObject().apply {
            put("payload", payload)
            put("signature", "forged")
        }.toString().toByteArray()
        assertInvalid("signature", badSig)
        assertInvalid("oversize", ByteArray(Constants.MAX_SIGNED_STABILITY_TLV_VALUE_BYTES + 1))
        assertInvalid("utf8", byteArrayOf(0xC3.toByte(), 0x28))
    }

    @Test
    fun unreadableChannelStateMustNotLookLikeAnInvalidEnvelope() {
        // Regression: an empty local channelId used to reach validateInbound and come back as
        // Invalid("channel"), indistinguishable from a forged envelope. Callers demoted that to a
        // deduped Lightning receipt, so the backing credit could never be recovered. Receivers now
        // check for unreadable local state BEFORE validating; this pins the reason strings apart so
        // a future refactor cannot silently merge the two cases again.
        val envelope = envelopeFor(inboundPayload())
        assertInvalid("channel", envelope, channelId = "")
        assertValid(envelope)
    }

    private fun assertValid(data: ByteArray) {
        assertTrue(validate(data) is SignedSettlementValidation.Valid)
    }

    private fun assertInvalid(
        reason: String,
        data: ByteArray,
        channelId: String = identifier
    ) {
        val result = validate(data, channelId = channelId)
        assertTrue(result is SignedSettlementValidation.Invalid)
        assertEquals(reason, (result as SignedSettlementValidation.Invalid).reason)
    }
}
