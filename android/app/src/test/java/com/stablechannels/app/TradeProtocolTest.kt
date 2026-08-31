package com.stablechannels.app

import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.TradeControlMessage
import com.stablechannels.app.services.TradeProtocol
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
class TradeProtocolTest {
    private val identifier = "ab".repeat(32)

    @Test
    fun requestHashUsesForwardLowercaseSha256OfExactBytes() {
        val payload = "{\"type\":\"TRADE_V1\",\"user_channel_id\":\"7\",\"expected_usd\":25.0}"
        assertEquals(
            "c07dcdff3aae2fc7ebd4fb19a7f1cd60b8e61c94a89acd35c5c600935d671602",
            TradeProtocol.requestHash(payload.toByteArray())
        )
    }

    @Test
    fun feeVectorsMatchRustWholeSatTruncation() {
        assertEquals(500_000L, TradeProtocol.expectedTradeFeeMsat(50.0, 99.5, 100_000.0))
        assertEquals(500_000L, TradeProtocol.expectedTradeFeeMsat(100.0, 50.0, 100_000.0))
        assertEquals(1_000L, TradeProtocol.expectedTradeFeeMsat(1.0, 0.0, 1_000_000.0))
        assertEquals(1L, TradeProtocol.expectedTradeFeeMsat(1.0, 0.1, 1_000_000.0))
        assertEquals(0.0, TradeProtocol.normalizeExpectedUsd(0.009), 0.0)
        assertEquals(1L, TradeProtocol.expectedTradeFeeMsat(0.0, 0.0, 100_000.0))
    }

    @Test
    fun preparedTradeContainsCorrelationQuoteTimestampAndStoredAllocation() {
        val sc = StableChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = USD(50.0),
            stableReceiverBTC = Bitcoin(100_000),
            backingSats = 55_000
        )
        val prepared = TradeProtocol.prepare(
            sc = sc,
            action = "sell",
            amountUsd = 10.0,
            amountBtc = 0.000099,
            feeUsd = 0.1,
            newExpectedUsd = 59.9,
            quotePrice = 100_000.0,
            now = 1_786_310_000L,
            tradeId = identifier
        )
        assertNotNull(prepared)
        prepared!!
        val payload = JSONObject(prepared.requestPayload)
        assertEquals("TRADE_V1", payload.getString("type"))
        assertEquals(identifier, payload.getString("channel_id"))
        assertEquals("7", payload.getString("user_channel_id"))
        assertEquals(identifier, payload.getString("trade_id"))
        assertEquals(100_000.0, payload.getDouble("quote_price"), 0.0)
        assertEquals(1_786_310_000L, payload.getLong("ts"))
        assertEquals(99_000L, prepared.feeMsat)
        assertEquals(64, prepared.requestHash.length)
        assertTrue(prepared.newBackingSats <= 100_000L - prepared.feeMsat / 1000L)
    }

    @Test
    fun signedControlRequiresCompleteCanonicalCorrelation() {
        val payload = JSONObject().apply {
            put("type", "SYNC_V1")
            put("channel_id", identifier)
            put("user_channel_id", "7")
            put("expected_usd", 25.0)
            put("backing_sats", 31_250)
            put("sync_version", 4)
            put("trade_id", identifier)
            put("trade_payment_id", identifier)
            put("request_hash", identifier)
        }.toString()
        val envelope = JSONObject().apply {
            put("payload", payload)
            put("signature", "valid")
        }.toString().toByteArray()
        val parsed = TradeProtocol.parseSignedControl(envelope, "peer") { bytes, signature, peer ->
            bytes.contentEquals(payload.toByteArray()) && signature == "valid" && peer == "peer"
        }
        assertTrue(parsed is TradeControlMessage.Sync)
        assertNotNull((parsed as TradeControlMessage.Sync).correlation)

        val partial = JSONObject(payload).apply { remove("request_hash") }.toString()
        val partialEnvelope = JSONObject().apply {
            put("payload", partial)
            put("signature", "valid")
        }.toString().toByteArray()
        assertNull(TradeProtocol.parseSignedControl(partialEnvelope, "peer") { _, _, _ -> true })
        assertNull(TradeProtocol.parseSignedControl(envelope, "peer") { _, _, _ -> false })

        val fractionalVersion = JSONObject(payload).apply { put("sync_version", 1.5) }.toString()
        val fractionalEnvelope = JSONObject().apply {
            put("payload", fractionalVersion)
            put("signature", "valid")
        }.toString().toByteArray()
        assertNull(TradeProtocol.parseSignedControl(fractionalEnvelope, "peer") { _, _, _ -> true })

        val booleanExpected = JSONObject(payload).apply { put("expected_usd", true) }.toString()
        val booleanEnvelope = JSONObject().apply {
            put("payload", booleanExpected)
            put("signature", "valid")
        }.toString().toByteArray()
        assertNull(TradeProtocol.parseSignedControl(booleanEnvelope, "peer") { _, _, _ -> true })
    }
}
