package com.stablechannels.app.services.websocket

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class MempoolWSBlock(
    val height: Int
)

@Serializable
data class MempoolWSVout(
    @SerialName("scriptpubkey_address") val scriptpubkeyAddress: String? = null,
    val value: Long? = null
)

@Serializable
data class MempoolWSVin(
    val txid: String? = null
)

@Serializable
data class MempoolWSTransaction(
    val txid: String,
    val vout: List<MempoolWSVout>? = null,
    val vin: List<MempoolWSVin>? = null
)

@Serializable
data class MempoolWSAddressTransactions(
    val mempool: List<MempoolWSTransaction>? = null,
    val confirmed: List<MempoolWSTransaction>? = null,
    val removed: List<MempoolWSTransaction>? = null
)

@Serializable
data class MempoolWSOutspend(
    val txid: String,
    val vin: Int
)

@Serializable
data class MempoolWSTxTrackingInfo(
    @SerialName("utxoSpent") val utxoSpent: Map<String, MempoolWSOutspend>? = null,
    val confirmed: Boolean? = null
)

@Serializable
data class MempoolWSMessage(
    val block: MempoolWSBlock? = null,
    val blocks: List<MempoolWSBlock>? = null,
    @SerialName("address-transactions") val addressTransactions: List<MempoolWSTransaction>? = null,
    @SerialName("block-transactions") val blockTransactions: List<MempoolWSTransaction>? = null,
    val address: String? = null,
    val txid: String? = null,
    @SerialName("multi-address-transactions") val multiAddressTransactions: Map<String, MempoolWSAddressTransactions>? = null,
    @SerialName("tracked-txs") val trackedTxs: Map<String, MempoolWSTxTrackingInfo>? = null
)
