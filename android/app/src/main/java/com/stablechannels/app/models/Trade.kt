package com.stablechannels.app.models

import java.util.Date

enum class TradeAction(val value: String) {
    BUY_BTC("buy"),
    SELL_BTC("sell");

    val displayName: String
        get() = when (this) {
            BUY_BTC -> "Buy BTC"
            SELL_BTC -> "Sell BTC"
        }

    companion object {
        fun fromString(s: String): TradeAction? = entries.find { it.value == s }
    }
}

data class PendingTrade(
    val action: TradeAction,
    val amountUSD: Double,
    val btcPrice: Double,
    val feeUSD: Double,
    val btcAmount: Double,
    val netAmountUSD: Double
)

data class PendingTradePayment(
    val newExpectedUSD: Double,
    val price: Double,
    val tradeDbId: Long,
    val action: String
)

data class PendingSplice(
    val direction: String, // "in" or "out"
    val amountSats: Long,
    val address: String? = null
)

data class ChannelRecord(
    val channelId: String,
    val userChannelId: String,
    val expectedUSD: Double,
    val note: String?,
    val backingSats: Long
)

data class TradeRecord(
    val id: Long,
    val channelId: String,
    val action: String,
    val amountUSD: Double,
    val amountBTC: Double,
    val btcPrice: Double,
    val feeUSD: Double,
    val paymentId: String?,
    val status: String,
    val createdAt: Long
) {
    val date: Date get() = Date(createdAt * 1000)
    val tradeAction: TradeAction? get() = TradeAction.fromString(action)
}

data class PaymentRecord(
    val id: Long,
    val paymentId: String?,
    val paymentType: String,
    val direction: String,
    val amountMsat: Long,
    val amountUSD: Double?,
    val btcPrice: Double?,
    val counterparty: String?,
    val status: String,
    val createdAt: Long,
    val feeMsat: Long = 0,
    val txid: String? = null,
    val address: String? = null,
    val confirmations: Int = 0
) {
    val date: Date get() = Date(createdAt * 1000)
    val amountSats: Long get() = amountMsat / 1000
    val isIncoming: Boolean get() = direction == "received"
}

data class PriceRecord(
    val id: Long,
    val price: Double,
    val source: String?,
    val timestamp: Long
) {
    val date: Date get() = Date(timestamp * 1000)
}

data class DailyPriceRecord(
    val date: String,
    val open: Double,
    val high: Double,
    val low: Double,
    val close: Double,
    val volume: Double?
)

data class OnchainTxRecord(
    val id: Long,
    val txid: String,
    val direction: String,
    val amountSats: Long,
    val address: String?,
    val btcPrice: Double?,
    val status: String,
    val confirmations: Int,
    val createdAt: Long
) {
    val date: Date get() = Date(createdAt * 1000)
}
