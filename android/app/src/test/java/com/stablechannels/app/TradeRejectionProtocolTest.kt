package com.stablechannels.app

import com.stablechannels.app.services.TradeControlMessage
import com.stablechannels.app.services.TradeProtocol
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TradeRejectionProtocolTest {
    // Wire vocabulary from src/trade.rs, emitted by stable_manager.rs.
    private val codes = listOf("invalid_amount", "stale_request", "invalid_fee", "invalid_quote",
        "quote_deviation", "insufficient_capacity", "settlement_required", "unsafe_allocation", "internal_failure")
    private val messages = mapOf(
        "invalid_amount" to "The amount is invalid. Review the amount and retry.",
        "stale_request" to "The quote expired before it could be accepted. Refresh and retry.",
        "invalid_fee" to "The fee was invalid. Refresh the quote before retrying.",
        "invalid_quote" to "A valid market quote is required. Refresh and retry.",
        "quote_deviation" to "The market moved outside the quote range. Refresh and retry.",
        "insufficient_capacity" to "The channel does not have enough capacity. Reduce the amount.",
        "settlement_required" to "Settle the current stability adjustment before retrying.",
        "unsafe_allocation" to "Cannot preserve the current channel allocation safely.",
        "internal_failure" to "The provider could not process. Try again later."
    )
    private fun payload(reason: String = "invalid_amount") = JSONObject().apply {
        put("type", "TRADE_REJECTED_V1")
        put("channel_id", "ab".repeat(32))
        put("trade_id", "cd".repeat(32))
        put("trade_payment_id", "ef".repeat(32))
        put("request_hash", "12".repeat(32))
        put("reason_code", reason)
        put("decided_at", 1_786_310_000L)
    }
    private fun envelope(payload: JSONObject) = JSONObject().apply {
        put("payload", payload.toString())
        put("signature", "signature")
    }.toString().toByteArray()

    @Test
    fun everyCurrentServerRejectionParsesAndHasDistinctLocalCopy() {
        for (code in codes) {
            val payload = payload(code)
            val message = TradeProtocol.parseSignedControl(envelope(payload), "provider") { bytes, sig, peer ->
                peer == "provider" && sig == "signature" && bytes.contentEquals(payload.toString().toByteArray())
            }
            assertTrue(code, message is TradeControlMessage.Rejected)
            assertEquals(code, (message as TradeControlMessage.Rejected).reasonCode)
        }
        assertEquals(codes.size, codes.map(TradeProtocol::rejectionMessage).toSet().size)
        assertEquals(messages, codes.associateWith(TradeProtocol::rejectionMessage))
    }

    @Test
    fun unknownCodesAndPeerProseAreNotAcceptedAsAResult() {
        val invalid = listOf(payload("future_code"), payload().put("message", "pay me"),
            payload().put("decided_at", -1), payload().put("decided_at", 1.5),
            payload().put("trade_id", "AB".repeat(32)), payload().apply { remove("request_hash") })
        for (payload in invalid) {
            var verified = false
            assertNull(TradeProtocol.parseSignedControl(envelope(payload), "provider") { _, _, _ -> verified = true; true })
            assertFalse("Invalid fields must be rejected before signature work", verified)
        }
    }

    @Test
    fun forgedAndUnverifiableResponsesCannotResolveATrade() {
        assertNull(TradeProtocol.parseSignedControl(envelope(payload()), "provider") { _, _, _ -> false })
        assertNull(TradeProtocol.parseSignedControl(envelope(payload()), "provider") { _, _, _ -> throw IllegalStateException("node unavailable") })
    }

    @Test
    fun oversizeMalformedUtf8AndCoercedEnvelopesFailBeforeVerification() {
        val invalid = listOf(ByteArray(TradeProtocol.MAX_CONTROL_TLV_BYTES + 1) { 32 },
            byteArrayOf(0xc3.toByte(), 0x28), "not JSON".toByteArray(),
            JSONObject().put("payload", payload()).put("signature", "signature").toString().toByteArray(),
            JSONObject().put("payload", payload().toString()).put("signature", 123).toString().toByteArray())
        for (data in invalid) {
            assertNull(TradeProtocol.parseSignedControl(data, "provider") { _, _, _ -> fail("Unexpected signature check"); true })
        }
    }
}
