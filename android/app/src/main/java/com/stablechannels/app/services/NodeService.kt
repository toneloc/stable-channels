package com.stablechannels.app.services

import android.content.Context
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import org.lightningdevkit.ldknode.*

class NodeService(private val context: Context) {

    var node: Node? = null
        private set
    var isRunning: Boolean = false
        private set
    var nodeId: String = ""
        private set
    var channels: List<ChannelDetails> = emptyList()
        private set

    private var eventJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO)

    private val _events = MutableSharedFlow<Event>(extraBufferCapacity = 64)
    val events: SharedFlow<Event> = _events

    fun start(network: Network, esploraURL: String, mnemonic: String?) {
        val dataDir = Constants.userDataDir(context)

        val config = Config()
        config.storageDirPath = dataDir.absolutePath
        config.network = network

        config.trustedPeers0conf = listOf(Constants.DEFAULT_LSP_PUBKEY)

        val anchorConfig = AnchorChannelsConfig()
        anchorConfig.trustedPeersNoReserve = listOf(Constants.DEFAULT_LSP_PUBKEY)
        anchorConfig.perChannelReserveSats = 25_000UL
        config.anchorChannelsConfig = anchorConfig

        val builder = Builder.fromConfig(config)
        builder.setEsploraServer(esploraURL)

        val rgsUrl = when (network) {
            Network.BITCOIN -> Constants.RGSServer.BITCOIN
            Network.SIGNET -> Constants.RGSServer.SIGNET
            Network.TESTNET -> Constants.RGSServer.TESTNET
            else -> Constants.RGSServer.BITCOIN
        }
        builder.setGossipSourceRgs(rgsUrl)

        builder.setLiquiditySourceLsps2(
            Constants.DEFAULT_LSP_ADDRESS,
            Constants.DEFAULT_LSP_PUBKEY,
            null
        )

        if (mnemonic != null) {
            builder.setEntropyBip39Mnemonic(Mnemonic(mnemonic))
        }

        val ldkNode = builder.build()
        ldkNode.start()

        node = ldkNode
        isRunning = true
        nodeId = ldkNode.nodeId()

        // Connect to gateway and LSP
        try { ldkNode.connect(Constants.DEFAULT_GATEWAY_PUBKEY, Constants.DEFAULT_GATEWAY_ADDRESS, true) } catch (_: Exception) {}
        try { ldkNode.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true) } catch (_: Exception) {}

        refreshChannels()
        startEventLoop()
    }

    fun stop() {
        eventJob?.cancel()
        eventJob = null
        node?.stop()
        node = null
        isRunning = false
    }

    private fun startEventLoop() {
        eventJob = scope.launch {
            val n = node ?: return@launch
            while (true) {
                val event = n.nextEventAsync()
                _events.emit(event)
                n.eventHandled()
            }
        }
    }

    fun refreshChannels() {
        channels = node?.listChannels() ?: emptyList()
    }

    fun connectAndOpenChannel(pubkey: String, address: String, amountSats: Long) {
        val n = node ?: throw NodeServiceError()
        n.connect(pubkey, address, true)
        n.openChannel(pubkey, address, amountSats.toULong(), null, null)
        refreshChannels()
    }

    fun closeChannel(userChannelId: String, counterpartyNodeId: String) {
        val n = node ?: throw NodeServiceError()
        n.closeChannel(userChannelId, counterpartyNodeId)
    }

    fun spliceIn(userChannelId: String, counterpartyNodeId: String, amountSats: Long) {
        val n = node ?: throw NodeServiceError()
        n.spliceIn(userChannelId, counterpartyNodeId, amountSats.toULong())
    }

    fun spliceOut(userChannelId: String, counterpartyNodeId: String, address: String, amountSats: Long) {
        val n = node ?: throw NodeServiceError()
        n.spliceOut(userChannelId, counterpartyNodeId, amountSats.toULong(), address)
    }

    fun sendPayment(invoice: Bolt11Invoice): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt11Payment().send(invoice, null)
    }

    fun sendBolt12(offer: Offer): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt12Payment().send(offer, null)
    }

    fun sendBolt12UsingAmount(offer: Offer, amountMsat: Long): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt12Payment().sendUsingAmount(offer, amountMsat.toULong(), null, null, null)
    }

    fun sendKeysend(amountMsat: Long, toNodeId: String): String {
        val n = node ?: throw NodeServiceError()
        return n.spontaneousPayment().send(amountMsat.toULong(), toNodeId, null, null)
    }

    fun sendKeysendWithTLV(amountMsat: Long, toNodeId: String, tlvs: List<CustomTlvRecord>): String {
        val n = node ?: throw NodeServiceError()
        return n.spontaneousPayment().sendWithCustomTlvs(amountMsat.toULong(), toNodeId, tlvs, null)
    }

    fun receivePayment(amountMsat: Long, description: String): Bolt11Invoice {
        val n = node ?: throw NodeServiceError()
        return n.bolt11Payment().receive(
            amountMsat.toULong(),
            Bolt11InvoiceDescription.Direct(description),
            Constants.INVOICE_EXPIRY_SECS.toUInt()
        )
    }

    fun receiveVariablePayment(description: String): Bolt11Invoice {
        val n = node ?: throw NodeServiceError()
        return n.bolt11Payment().receiveVariableAmount(
            Bolt11InvoiceDescription.Direct(description),
            Constants.INVOICE_EXPIRY_SECS.toUInt()
        )
    }

    fun receiveViaJitChannel(amountMsat: Long, description: String): Bolt11Invoice {
        val n = node ?: throw NodeServiceError()
        return n.bolt11Payment().receiveViaJitChannel(
            amountMsat.toULong(),
            Bolt11InvoiceDescription.Direct(description),
            Constants.INVOICE_EXPIRY_SECS.toUInt(),
            null
        )
    }

    fun newOnchainAddress(): String {
        val n = node ?: throw NodeServiceError()
        return n.onchainPayment().newAddress()
    }

    fun sendOnchain(address: String, amountSats: Long): String {
        val n = node ?: throw NodeServiceError()
        return n.onchainPayment().sendToAddress(address, amountSats.toULong(), null)
    }

    fun sendAllOnchain(address: String): String {
        val n = node ?: throw NodeServiceError()
        return n.onchainPayment().sendAllToAddress(address, false)
    }

    fun balances(): BalanceDetails? = node?.listBalances()

    fun spendableOnchainSats(): Long {
        val b = balances() ?: return 0
        return b.spendableOnchainBalanceSats.toLong()
    }

    fun totalOnchainSats(): Long {
        val b = balances() ?: return 0
        return b.totalOnchainBalanceSats.toLong()
    }

    fun signMessage(message: ByteArray): String {
        val n = node ?: throw NodeServiceError()
        return n.signMessage(message.toList())
    }

    fun verifySignature(message: ByteArray, signature: String, pubkey: String): Boolean {
        return try {
            val n = node ?: return false
            n.verifySignature(message.toList(), signature, pubkey)
        } catch (_: Exception) { false }
    }

    class NodeServiceError : Exception("Node not running")
}
