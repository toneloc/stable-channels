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
    val btcAmount: Double,
    val tradeDbId: Long
)

class TradeService(
    private val nodeService: NodeService,
    private val databaseService: DatabaseService
) {
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
        return preparePersistAndSend(
            sc, "buy", amountUSD, btcAmount, feeUSD, newExpectedUSD, price
        )
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
        return preparePersistAndSend(
            sc, "sell", amountUSD, btcAmount, feeUSD, newExpectedUSD, price
        )
    }

    private fun preparePersistAndSend(
        sc: StableChannel,
        action: String,
        amountUsd: Double,
        amountBtc: Double,
        feeUsd: Double,
        newExpectedUsd: Double,
        price: Double
    ): TradeResult? {
        val prepared = TradeProtocol.prepare(
            sc = sc,
            action = action,
            amountUsd = amountUsd,
            amountBtc = amountBtc,
            feeUsd = feeUsd,
            newExpectedUsd = newExpectedUsd,
            quotePrice = price
        ) ?: return null

        // This row is the recovery authority. It must exist before the non-refundable fee send.
        val tradeDbId = databaseService.recordPreparedTrade(prepared)
        val paymentId = try {
            val signature = nodeService.signMessage(
                prepared.requestPayload.toByteArray(Charsets.UTF_8)
            )
            val envelope = JSONObject().apply {
                put("payload", prepared.requestPayload)
                put("signature", signature)
            }.toString().toByteArray(Charsets.UTF_8)
            nodeService.sendKeysendWithTLV(
                prepared.feeMsat,
                sc.counterparty,
                listOf(CustomTlvRecord(Constants.STABLE_CHANNEL_TLV_TYPE.toULong(), envelope))
            )
        } catch (error: Exception) {
            databaseService.markTradeSendFailed(tradeDbId)
            throw error
        }

        // The payment has left the node at this point. A local bookkeeping failure must not
        // report a send failure (or invite the user to pay the non-refundable fee twice).
        val attached = try {
            databaseService.attachTradePaymentId(tradeDbId, paymentId)
        } catch (error: Exception) {
            false
        }
        if (!attached) {
            AuditService.log("TRADE_PAYMENT_ID_PERSIST_FAILED", mapOf(
                "trade_db_id" to tradeDbId,
                "trade_id" to prepared.tradeId,
                "payment_id" to paymentId
            ))
        }
        AuditService.log("TRADE_MESSAGE_SENT", mapOf(
            "trade_id" to prepared.tradeId,
            "request_hash" to prepared.requestHash,
            "payment_id" to paymentId,
            "fee_msat" to prepared.feeMsat,
            "new_expected_usd" to prepared.newExpectedUsd,
            "new_backing_sats" to prepared.newBackingSats
        ))
        return TradeResult(paymentId, prepared.newExpectedUsd, amountBtc, tradeDbId)
    }
}
