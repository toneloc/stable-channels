package com.stablechannels.app.services

import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.util.Constants
import org.json.JSONObject
import org.lightningdevkit.ldknode.CustomTlvRecord
import kotlin.math.max
import com.stablechannels.app.models.Bitcoin

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
    private fun liveSnapshot(sc: StableChannel, price: Double): StabilizationSnapshot? {
        val channel = nodeService.node?.listChannels()?.firstOrNull { it.userChannelId == sc.userChannelId && it.isChannelReady }
            ?: return null
        val capacity = (channel.outboundCapacityMsat / 1000u).toLong()
        return StabilizationSnapshot(capacity + (channel.unspendablePunishmentReserve?.toLong() ?: 0L),
            capacity, sc.backingSats, sc.expectedUSD.amount, price)
    }

    fun maxSellCents(sc: StableChannel, price: Double): Long = liveSnapshot(sc, price)?.maxOrderCents() ?: 0L

    fun executeBuy(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double
    ): TradeResult {
        if (!amountUSD.isFinite() || amountUSD <= 0 || amountUSD > sc.expectedUSD.amount || !price.isFinite() || price <= 0)
            throw TradeValidationException("Enter a positive amount within your stabilized USD balance and use a fresh quote")
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
        price: Double
    ): TradeResult {
        if (!amountUSD.isFinite() || amountUSD <= 0 || !price.isFinite() || price <= 0)
            throw TradeValidationException("Enter a positive amount and use a fresh BTC/USD quote")
        val netAmount = amountUSD - feeUSD
        val newExpectedUSD = sc.expectedUSD.amount + netAmount
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
    ): TradeResult {
        val snapshot = liveSnapshot(sc, price)
            ?: throw TradeValidationException("The live channel balance is unavailable. Retry when the channel is ready.")
        if (newExpectedUsd > sc.expectedUSD.amount && !snapshot.accepts(kotlin.math.floor(amountUsd * 100 + 1e-7).toLong()))
            throw TradeValidationException(StabilizationPolicy.limitExceededMessage(snapshot.maxOrderCents()))
        val liveSc = sc.copy(stableReceiverBTC = Bitcoin(snapshot.receiverSats))
        val prepared = TradeProtocol.prepare(
            sc = liveSc,
            spendableSats = snapshot.spendableSats,
            action = action,
            amountUsd = amountUsd,
            amountBtc = amountBtc,
            feeUsd = feeUsd,
            newExpectedUsd = newExpectedUsd,
            quotePrice = price
        ) ?: throw TradeValidationException("This trade cannot preserve the current channel allocation safely. Settle the stability adjustment and retry.")

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
