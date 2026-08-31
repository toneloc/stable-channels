package com.stablechannels.app.services

import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.util.Constants
import org.json.JSONObject
import java.security.MessageDigest
import java.security.SecureRandom
import kotlin.math.abs
import kotlin.math.floor

data class TradeCorrelation(
    val tradeId: String,
    val tradePaymentId: String,
    val requestHash: String
)

sealed interface TradeControlMessage {
    data class Sync(
        val channelId: String,
        val userChannelId: String,
        val expectedUsd: Double,
        val backingSats: Long,
        val syncVersion: Long,
        val correlation: TradeCorrelation?
    ) : TradeControlMessage

    data class Rejected(
        val channelId: String,
        val correlation: TradeCorrelation,
        val reasonCode: String,
        val decidedAt: Long
    ) : TradeControlMessage
}

data class PreparedTrade(
    val channelId: String,
    val userChannelId: String,
    val tradeId: String,
    val requestHash: String,
    val requestPayload: String,
    val action: String,
    val amountUsd: Double,
    val amountBtc: Double,
    val feeUsd: Double,
    val feeMsat: Long,
    val oldExpectedUsd: Double,
    val newExpectedUsd: Double,
    val newBackingSats: Long,
    val quotePrice: Double,
    val createdAt: Long,
    val expiresAt: Long
)

object TradeProtocol {
    const val RESULT_CONTROL_AMOUNT_MSAT = 1L
    const val RESULT_TIMEOUT_SECS = 15L * 60L
    const val RESPONSE_RETRY_WINDOW_SECS = 14L * 24L * 60L * 60L
    private val rejectionReasons = setOf(
        "invalid_amount",
        "stale_request",
        "invalid_fee",
        "invalid_quote",
        "quote_deviation",
        "insufficient_capacity",
        "settlement_required",
        "unsafe_allocation",
        "internal_failure"
    )

    fun normalizeExpectedUsd(value: Double): Double =
        if (value.isFinite() && value >= 0.0 && value < 0.01) 0.0 else value

    fun requestHash(payload: ByteArray): String = MessageDigest
        .getInstance("SHA-256")
        .digest(payload)
        .joinToString("") { "%02x".format(it) }

    fun expectedTradeFeeMsat(
        oldExpectedUsd: Double,
        newExpectedUsd: Double,
        quotePrice: Double
    ): Long? {
        val feeRate = Constants.STABLE_CHANNEL_TRADE_FEE_RATE
        if (!oldExpectedUsd.isFinite() || oldExpectedUsd < 0.0 ||
            !newExpectedUsd.isFinite() || newExpectedUsd < 0.0 ||
            !quotePrice.isFinite() || quotePrice <= 0.0 ||
            !feeRate.isFinite() || feeRate < 0.0 || feeRate >= 1.0
        ) return null

        val targetDelta = abs(newExpectedUsd - oldExpectedUsd)
        val grossUsd = if (newExpectedUsd > oldExpectedUsd) {
            targetDelta / (1.0 - feeRate)
        } else {
            targetDelta
        }
        val feeSats = grossUsd * feeRate / quotePrice * Constants.SATS_IN_BTC.toDouble()
        if (!feeSats.isFinite() || feeSats < 0.0 || feeSats > Long.MAX_VALUE / 1000.0) {
            return null
        }
        return (feeSats.toLong() * 1000L).coerceAtLeast(1L)
    }

    fun prepare(
        sc: StableChannel,
        action: String,
        amountUsd: Double,
        amountBtc: Double,
        feeUsd: Double,
        newExpectedUsd: Double,
        quotePrice: Double,
        now: Long = System.currentTimeMillis() / 1000L,
        tradeId: String = randomIdentifier()
    ): PreparedTrade? {
        val normalizedExpected = normalizeExpectedUsd(newExpectedUsd)
        if (!isCanonicalIdentifier(sc.channelId) || sc.userChannelId.isBlank() ||
            !isCanonicalIdentifier(tradeId) || !amountUsd.isFinite() || amountUsd <= 0.0 ||
            !amountBtc.isFinite() || amountBtc < 0.0 || !feeUsd.isFinite() || feeUsd < 0.0
        ) return null
        val feeMsat = expectedTradeFeeMsat(sc.expectedUSD.amount, normalizedExpected, quotePrice)
            ?: return null
        val feeSats = feeMsat / 1000L
        val postFeeReceiver = sc.stableReceiverBTC.sats - feeSats
        if (postFeeReceiver < 0L) return null
        val backing = tradeBackingAfterDelta(
            receiverSats = postFeeReceiver,
            currentBackingSats = sc.backingSats,
            currentExpectedUsd = sc.expectedUSD.amount,
            newExpectedUsd = normalizedExpected,
            price = quotePrice
        ) ?: return null

        val payload = JSONObject().apply {
            put("type", Constants.TRADE_MESSAGE_TYPE)
            put("channel_id", sc.channelId)
            put("user_channel_id", sc.userChannelId)
            put("trade_id", tradeId)
            put("expected_usd", normalizedExpected)
            put("quote_price", quotePrice)
            put("ts", now)
        }.toString()
        return PreparedTrade(
            channelId = sc.channelId,
            userChannelId = sc.userChannelId,
            tradeId = tradeId,
            requestHash = requestHash(payload.toByteArray(Charsets.UTF_8)),
            requestPayload = payload,
            action = action,
            amountUsd = amountUsd,
            amountBtc = amountBtc,
            feeUsd = feeUsd,
            feeMsat = feeMsat,
            oldExpectedUsd = sc.expectedUSD.amount,
            newExpectedUsd = normalizedExpected,
            newBackingSats = backing,
            quotePrice = quotePrice,
            createdAt = now,
            expiresAt = now + RESULT_TIMEOUT_SECS
        )
    }

    fun tradeBackingAfterDelta(
        receiverSats: Long,
        currentBackingSats: Long,
        currentExpectedUsd: Double,
        newExpectedUsd: Double,
        price: Double
    ): Long? {
        val normalizedExpected = normalizeExpectedUsd(newExpectedUsd)
        if (receiverSats < 0L || currentBackingSats < 0L ||
            !currentExpectedUsd.isFinite() || currentExpectedUsd < 0.0 ||
            !normalizedExpected.isFinite() || normalizedExpected < 0.0 ||
            !price.isFinite() || price <= 0.0
        ) return null
        val receiverUsd = receiverSats.toDouble() / Constants.SATS_IN_BTC.toDouble() * price
        if (normalizedExpected > receiverUsd) return null
        if (normalizedExpected == 0.0) {
            return if (!allocationDriftIsActionable(currentBackingSats, currentExpectedUsd, price)) 0L else null
        }

        val currentTarget = currentExpectedUsd / price * Constants.SATS_IN_BTC.toDouble()
        val newTarget = normalizedExpected / price * Constants.SATS_IN_BTC.toDouble()
        if (!currentTarget.isFinite() || !newTarget.isFinite() ||
            currentTarget < 0.0 || newTarget < 0.0 ||
            currentTarget >= Long.MAX_VALUE.toDouble() || newTarget >= Long.MAX_VALUE.toDouble()
        ) return null
        val currentTargetSats = floor(currentTarget).toLong()
        val newTargetSats = floor(newTarget).toLong()
        var backing = try {
            if (normalizedExpected >= currentExpectedUsd) {
                Math.addExact(currentBackingSats, Math.subtractExact(newTargetSats, currentTargetSats))
            } else {
                Math.subtractExact(currentBackingSats, Math.subtractExact(currentTargetSats, newTargetSats))
            }
        } catch (_: ArithmeticException) {
            return null
        }
        if (backing < 0L) return null
        if (currentExpectedUsd < 0.01 && currentBackingSats == 0L && backing <= receiverSats) {
            val nativeUsd = (receiverSats - backing).toDouble() / Constants.SATS_IN_BTC.toDouble() * price
            if (nativeUsd < 0.01) backing = receiverSats
        }
        return backing.takeIf { it > 0L && it <= receiverSats }
    }

    fun parseSignedControl(
        data: ByteArray,
        expectedCounterparty: String,
        verifySignature: (ByteArray, String, String) -> Boolean
    ): TradeControlMessage? {
        return try {
            val envelope = JSONObject(String(data, Charsets.UTF_8))
            val payloadStr = envelope.getString("payload")
            val signature = envelope.getString("signature")
            val payloadBytes = payloadStr.toByteArray(Charsets.UTF_8)
            if (!verifySignature(payloadBytes, signature, expectedCounterparty)) return null
            val payload = JSONObject(payloadStr)
            when (payload.optString("type")) {
                Constants.SYNC_MESSAGE_TYPE -> parseSync(payload)
                Constants.TRADE_REJECTED_MESSAGE_TYPE -> parseRejection(payload)
                else -> null
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun parseSync(payload: JSONObject): TradeControlMessage.Sync? {
        val channelId = payload.optString("channel_id")
        val userChannelId = payload.optString("user_channel_id")
        val expectedUsd = jsonDouble(payload, "expected_usd")?.let(::normalizeExpectedUsd)
            ?: return null
        val backingSats = jsonInteger(payload, "backing_sats") ?: return null
        val syncVersion = jsonInteger(payload, "sync_version") ?: return null
        if (!isCanonicalIdentifier(channelId) || userChannelId.isBlank() ||
            !expectedUsd.isFinite() || expectedUsd < 0.0 || backingSats < 0L || syncVersion <= 0L
        ) return null

        val correlationKeys = listOf("trade_id", "trade_payment_id", "request_hash")
        val present = correlationKeys.count { payload.has(it) && !payload.isNull(it) }
        val correlation = when (present) {
            0 -> null
            3 -> TradeCorrelation(
                payload.getString("trade_id"),
                payload.getString("trade_payment_id"),
                payload.getString("request_hash")
            ).takeIf {
                isCanonicalIdentifier(it.tradeId) && isCanonicalIdentifier(it.tradePaymentId) &&
                    isCanonicalIdentifier(it.requestHash)
            } ?: return null
            else -> return null
        }
        return TradeControlMessage.Sync(
            channelId, userChannelId, expectedUsd, backingSats, syncVersion, correlation
        )
    }

    private fun parseRejection(payload: JSONObject): TradeControlMessage.Rejected? {
        val allowed = setOf(
            "type", "channel_id", "trade_id", "trade_payment_id", "request_hash",
            "reason_code", "decided_at"
        )
        if (payload.keys().asSequence().any { it !in allowed }) return null
        val channelId = payload.getString("channel_id")
        val correlation = TradeCorrelation(
            payload.getString("trade_id"),
            payload.getString("trade_payment_id"),
            payload.getString("request_hash")
        )
        val reason = payload.getString("reason_code")
        val decidedAt = jsonInteger(payload, "decided_at") ?: return null
        if (!isCanonicalIdentifier(channelId) || !isCanonicalIdentifier(correlation.tradeId) ||
            !isCanonicalIdentifier(correlation.tradePaymentId) ||
            !isCanonicalIdentifier(correlation.requestHash) || reason !in rejectionReasons || decidedAt < 0L
        ) return null
        return TradeControlMessage.Rejected(channelId, correlation, reason, decidedAt)
    }

    fun rejectionMessage(reason: String): String = when (reason) {
        "invalid_amount" -> "The trade amount is invalid. Review the amount and retry."
        "stale_request" -> "The quote expired before it could be accepted. Refresh and retry."
        "invalid_fee" -> "The trade fee was invalid. Refresh the quote before retrying."
        "invalid_quote" -> "A valid market quote is required. Refresh and retry."
        "quote_deviation" -> "The market moved outside the quote range. Refresh and retry."
        "insufficient_capacity" -> "The channel does not have enough capacity for this trade. Reduce the amount."
        "settlement_required" -> "Settle the current stability adjustment before retrying this trade."
        "unsafe_allocation" -> "This trade cannot preserve the current channel allocation safely."
        else -> "The provider could not process the trade. Try again later."
    }

    fun isCanonicalIdentifier(value: String): Boolean =
        value.length == 64 && value.all { it in '0'..'9' || it in 'a'..'f' }

    private fun jsonDouble(payload: JSONObject, key: String): Double? =
        (payload.opt(key) as? Number)?.toDouble()?.takeIf { it.isFinite() }

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

    private fun allocationDriftIsActionable(backingSats: Long, expectedUsd: Double, price: Double): Boolean {
        val currentValue = backingSats.toDouble() / Constants.SATS_IN_BTC.toDouble() * price
        val driftUsd = abs(currentValue - expectedUsd)
        if (expectedUsd < 0.01) return driftUsd >= Constants.STABILITY_THRESHOLD_USD
        val driftPercent = driftUsd / expectedUsd * 100.0
        return driftUsd >= Constants.STABILITY_THRESHOLD_USD &&
            driftPercent >= Constants.STABILITY_THRESHOLD_PERCENT
    }

    private fun randomIdentifier(): String = ByteArray(32)
        .also { SecureRandom().nextBytes(it) }
        .joinToString("") { "%02x".format(it) }
}
