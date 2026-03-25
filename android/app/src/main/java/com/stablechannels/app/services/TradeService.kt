package com.stablechannels.app.services

import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.util.Constants
import org.json.JSONObject
import org.lightningdevkit.ldknode.CustomTlvRecord
import kotlin.math.max
import kotlin.math.min

data class TradeResult(
    val paymentId: String,
    val newExpectedUSD: Double,
    val btcAmount: Double
)

class TradeService(private val nodeService: NodeService) {

    fun executeBuy(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double
    ): TradeResult? {
        if (amountUSD <= 0 || amountUSD > sc.expectedUSD.amount || price <= 0) return null
        val netAmount = amountUSD - feeUSD
        val newExpectedUSD = max(sc.expectedUSD.amount - amountUSD, 0.0)
        val btcAmount = netAmount / price
        val paymentId = sendTradeMessage(newExpectedUSD, sc.userChannelId, sc.channelId, sc.counterparty, feeUSD, price)
        return TradeResult(paymentId, newExpectedUSD, btcAmount)
    }

    fun executeSell(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double,
        maxUSD: Double
    ): TradeResult? {
        if (amountUSD <= 0 || price <= 0) return null
        val netAmount = amountUSD - feeUSD
        val newExpectedUSD = min(sc.expectedUSD.amount + netAmount, maxUSD)
        val btcAmount = netAmount / price
        val paymentId = sendTradeMessage(newExpectedUSD, sc.userChannelId, sc.channelId, sc.counterparty, feeUSD, price)
        return TradeResult(paymentId, newExpectedUSD, btcAmount)
    }

    fun sendTradeMessage(expectedUSD: Double, userChannelId: String, channelId: String, counterparty: String, feeUSD: Double, price: Double): String {
        val payload = JSONObject().apply {
            put("type", Constants.TRADE_MESSAGE_TYPE)
            put("user_channel_id", userChannelId)
            put("channel_id", channelId)
            put("expected_usd", expectedUSD)
        }
        val payloadStr = payload.toString()
        val signature = nodeService.signMessage(payloadStr.toByteArray(Charsets.UTF_8))

        val envelope = JSONObject().apply {
            put("payload", payloadStr)
            put("signature", signature)
        }
        val envelopeBytes = envelope.toString().toByteArray(Charsets.UTF_8)

        val feeMsat = max((feeUSD / price * Constants.SATS_IN_BTC).toLong() * 1000, 1)
        val tlv = CustomTlvRecord(Constants.STABLE_CHANNEL_TLV_TYPE.toULong(), envelopeBytes.map { it.toUByte() })
        return nodeService.sendKeysendWithTLV(feeMsat, counterparty, listOf(tlv))
    }

    companion object {
        fun parseIncomingTLV(
            data: ByteArray,
            expectedCounterparty: String,
            verifySignature: (ByteArray, String, String) -> Boolean
        ): Triple<String, Double, String>? {
            return try {
                val envelopeStr = String(data, Charsets.UTF_8)
                val envelope = JSONObject(envelopeStr)
                val payloadStr = envelope.getString("payload")
                val signature = envelope.getString("signature")

                if (!verifySignature(payloadStr.toByteArray(Charsets.UTF_8), signature, expectedCounterparty)) {
                    return null
                }

                val payload = JSONObject(payloadStr)
                val type = payload.getString("type")
                val expectedUsd = payload.getDouble("expected_usd")
                val userChannelId = payload.getString("user_channel_id")

                Triple(type, expectedUsd, userChannelId)
            } catch (_: Exception) {
                null
            }
        }
    }
}
