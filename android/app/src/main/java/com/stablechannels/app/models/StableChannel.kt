package com.stablechannels.app.models

import com.stablechannels.app.util.Constants
import kotlinx.serialization.Serializable
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToLong

@Serializable
data class Bitcoin(val sats: Long = 0) {
    companion object {
        val ZERO = Bitcoin(0)

        fun fromSats(sats: Long) = Bitcoin(sats)

        fun fromBTC(btc: Double): Bitcoin {
            val sats = (btc * Constants.SATS_IN_BTC).roundToLong()
            return Bitcoin(sats)
        }

        fun fromUSD(usd: USD, price: Double): Bitcoin {
            val btc = usd.amount / price
            return fromBTC(btc)
        }
    }

    fun toBTC(): Double = sats.toDouble() / Constants.SATS_IN_BTC

    val formatted: String
        get() = String.format(Locale.US, "%.8f BTC", toBTC())
}

@Serializable
data class USD(val amount: Double = 0.0) {
    companion object {
        val ZERO = USD(0.0)

        fun fromBitcoin(btc: Bitcoin, price: Double): USD {
            return USD(btc.toBTC() * price)
        }
    }

    fun toMsats(price: Double): Long {
        val btcValue = amount / price
        val sats = btcValue * Constants.SATS_IN_BTC
        val millisats = sats * 1000.0
        return abs(millisats).toLong()
    }

    val formatted: String
        get() = String.format(Locale.US, "$%.2f", amount)
}

@Serializable
data class StableChannel(
    var channelId: String = "",
    var userChannelId: String = "",
    var isStableReceiver: Boolean = true,
    var counterparty: String = Constants.DEFAULT_LSP_PUBKEY,
    var expectedUSD: USD = USD.ZERO,
    var expectedBTC: Bitcoin = Bitcoin.ZERO,
    var stableReceiverBTC: Bitcoin = Bitcoin.ZERO,
    var stableProviderBTC: Bitcoin = Bitcoin.ZERO,
    var stableReceiverUSD: USD = USD.ZERO,
    var stableProviderUSD: USD = USD.ZERO,
    var riskLevel: Int = 0,
    var timestamp: Long = System.currentTimeMillis() / 1000,
    var formattedDatetime: String = "",
    var paymentMade: Boolean = false,
    var scDir: String = ".data",
    var latestPrice: Double = 0.0,
    var prices: String = "",
    var onchainBTC: Bitcoin = Bitcoin.ZERO,
    var onchainUSD: USD = USD.ZERO,
    var note: String? = null,
    var nativeChannelBTC: Bitcoin = Bitcoin.ZERO,
    var backingSats: Long = 0,
    var lastStabilityPayment: Long = 0
) {
    companion object {
        val DEFAULT = StableChannel()
    }
}
