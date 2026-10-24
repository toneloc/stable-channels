package com.stablechannels.app.services.send

import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.DecodedBolt12
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import com.stablechannels.app.domain.send.OutboundPaymentStatus
import com.stablechannels.app.domain.send.SendBalanceSnapshot
import com.stablechannels.app.domain.send.SendNode
import com.stablechannels.app.domain.send.SendPaymentRecorder
import com.stablechannels.app.domain.send.SendWallet
import com.stablechannels.app.domain.send.SpliceChannelCandidate

internal class FakeSendWallet(
    override var network: BitcoinNetwork? = BitcoinNetwork.TESTNET,
    override var isOnline: Boolean = true,
    var price: Double = 100_000.0,
    var snapshot: SendBalanceSnapshot = balances(),
    var outbound: Map<String, OutboundPaymentStatus> = emptyMap(),
) : SendWallet {
    override fun accountingPrice(): Double = price

    override fun balances(): SendBalanceSnapshot = snapshot

    override fun outboundPaymentStatus(paymentHash: String): OutboundPaymentStatus? =
        outbound[paymentHash]
}

internal fun balances(
    lightningSats: Long = 0,
    onchainSats: Long = 0,
    channels: List<SpliceChannelCandidate> = emptyList(),
    isSweeping: Boolean = false,
) =
    SendBalanceSnapshot(
        channels,
        lightningSats,
        onchainSats,
        lightningSats + onchainSats,
        isSweeping,
    )

internal fun readyChannel(outboundSats: Long, id: String = "chan") =
    SpliceChannelCandidate(id, "user-$id", "peer-$id", outboundSats * 1000, isReady = true)

/** Records every node call as text, in order. */
internal class FakeSendNode : SendNode {
    val calls = mutableListOf<String>()
    var paymentId = "pid-1"
    var txid = "txid-1"
    var failWith: Exception? = null
    var spliceFailure: Exception? = null
    /** Runs inside a Lightning dispatch, before it returns; simulates a fast settlement. */
    var onLightningDispatch: (String) -> Unit = {}

    private fun call(text: String) {
        calls += text
        failWith?.let { throw it }
    }

    override fun payBolt11(invoice: String, amountMsat: Long?): String {
        call("payBolt11 $invoice $amountMsat")
        onLightningDispatch(paymentId)
        return paymentId
    }

    override fun payBolt12(offer: String, amountMsat: Long): String {
        call("payBolt12 $offer $amountMsat")
        onLightningDispatch(paymentId)
        return paymentId
    }

    override fun sendOnchain(address: String, amountSats: Long, feeRateSatVb: Double?): String {
        call("sendOnchain $address $amountSats $feeRateSatVb")
        return txid
    }

    override fun sendAllOnchain(address: String, feeRateSatVb: Double?): String {
        call("sendAllOnchain $address $feeRateSatVb")
        return txid
    }

    override fun spliceOut(channel: SpliceChannelCandidate, address: String, amountSats: Long) {
        calls += "spliceOut ${channel.channelId} $address $amountSats"
        spliceFailure?.let { throw it }
    }

    override fun ensureNoUnsettledSurplus(amountMsat: Long) {
        calls += "surplusCheck $amountMsat"
    }

    override fun beginSpliceOut(amountSats: Long, address: String, price: Double) {
        calls += "beginSpliceOut $amountSats"
    }

    override fun cancelSpliceStart() {
        calls += "cancelSpliceStart"
    }

    override fun onchainBroadcast(amountSats: Long, isSendAll: Boolean, txid: String) {
        calls += "broadcast $amountSats $isSendAll $txid"
    }
}

internal class FakeSendRecorder : SendPaymentRecorder {
    val rows = mutableListOf<String>()

    override fun recordLightning(
        paymentId: String,
        paymentType: String,
        amountMsat: Long,
        price: Double,
    ) {
        rows += "$paymentType $paymentId $amountMsat"
    }

    override fun recordOnchain(txid: String, amountMsat: Long, price: Double, address: String) {
        rows += "onchain $txid $amountMsat $address"
    }
}

/** Decodes only the strings registered here; JVM tests cannot load the node library. */
internal class FakeInvoiceDecoder(
    val invoices: MutableMap<String, DecodedBolt11> = mutableMapOf(),
    val offers: MutableMap<String, DecodedBolt12> = mutableMapOf(),
) : LightningInvoiceDecoder {
    override fun decodeBolt11(raw: String): DecodedBolt11? = invoices[raw]

    override fun decodeBolt12(raw: String): DecodedBolt12? = offers[raw]
}
