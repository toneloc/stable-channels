package com.stablechannels.app.services.send

import com.stablechannels.app.AppState
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.OutboundPaymentStatus
import com.stablechannels.app.domain.send.SendBalanceSnapshot
import com.stablechannels.app.domain.send.SendNode
import com.stablechannels.app.domain.send.SendPaymentRecorder
import com.stablechannels.app.domain.send.SendWallet
import com.stablechannels.app.domain.send.SpliceChannelCandidate
import com.stablechannels.app.services.toDomain
import com.stablechannels.app.util.Constants
import org.lightningdevkit.ldknode.Bolt11Invoice
import org.lightningdevkit.ldknode.ChannelDetails
import org.lightningdevkit.ldknode.Offer

/** The send flow's ports, backed by the running wallet. */
class AppStateSendAdapter(private val appState: AppState) :
    SendWallet, SendNode, SendPaymentRecorder {
    private val nodeService
        get() = appState.nodeService

    override val network: BitcoinNetwork?
        get() = nodeService.activeNetwork?.toDomain()

    override val isOnline: Boolean
        get() = appState.isOnline.value

    override fun accountingPrice(): Double = appState.priceService.currentAccountingPrice()

    override fun balances(): SendBalanceSnapshot =
        SendBalanceSnapshot(
            channels = nodeService.channels.map { it.toCandidate() },
            lightningSats = appState.lightningBalanceSats.value,
            onchainSats = appState.spendableOnchainSats.value,
            totalSats = appState.totalBalanceSats.value,
            isSweeping = appState.isSpliceInFlight,
        )

    override fun outboundPaymentStatus(paymentHash: String): OutboundPaymentStatus? =
        nodeService.outboundPaymentStatus(paymentHash)

    override fun payBolt11(invoice: String, amountMsat: Long?): String {
        val parsed = Bolt11Invoice.fromStr(invoice)
        return if (amountMsat == null) nodeService.sendPayment(parsed)
        else nodeService.sendPaymentUsingAmount(parsed, amountMsat)
    }

    override fun payBolt12(offer: String, amountMsat: Long): String =
        nodeService.sendBolt12UsingAmount(Offer.fromStr(offer), amountMsat)

    override fun sendOnchain(address: String, amountSats: Long, feeRateSatVb: Double?): String =
        nodeService.sendOnchain(address, amountSats, feeRateSatVb)

    override fun sendAllOnchain(address: String, feeRateSatVb: Double?): String =
        nodeService.sendAllOnchain(address, feeRateSatVb)

    override fun spliceOut(channel: SpliceChannelCandidate, address: String, amountSats: Long) =
        nodeService.spliceOut(
            channel.userChannelId,
            channel.counterpartyNodeId,
            address,
            amountSats,
        )

    override fun ensureNoUnsettledSurplus(amountMsat: Long) =
        appState.ensureNoUnsettledSurplus(amountMsat)

    override fun beginSpliceOut(amountSats: Long, address: String, price: Double) =
        appState.beginSpliceOut(amountSats, address, price)

    override fun cancelSpliceStart() = appState.cancelPendingSpliceStart()

    override fun onchainBroadcast(amountSats: Long, isSendAll: Boolean, txid: String) =
        appState.onchainSendBroadcasted(amountSats, isSendAll, txid)

    override fun recordLightning(
        paymentId: String,
        paymentType: String,
        amountMsat: Long,
        price: Double,
    ) = appState.recordOutgoingLightningPayment(paymentId, paymentType, amountMsat, price)

    override fun recordOnchain(txid: String, amountMsat: Long, price: Double, address: String) {
        appState.databaseService?.recordPayment(
            paymentId = null,
            paymentType = "onchain",
            direction = "sent",
            amountMsat = amountMsat,
            amountUSD =
                (amountMsat.toDouble() / 1000 / Constants.SATS_IN_BTC * price).takeIf { price > 0 },
            btcPrice = price.takeIf { it > 0 },
            status = "pending",
            txid = txid,
            address = address,
        )
        appState.notifyPaymentRecorded()
    }
}

private fun ChannelDetails.toCandidate() =
    SpliceChannelCandidate(
        channelId = channelId,
        userChannelId = userChannelId,
        counterpartyNodeId = counterpartyNodeId,
        outboundCapacityMsat = outboundCapacityMsat.toLong(),
        isReady = isChannelReady,
    )
