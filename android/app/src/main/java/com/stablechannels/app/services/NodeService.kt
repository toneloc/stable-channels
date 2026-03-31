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
import java.io.File

class NodeService(private val context: Context) {

    var node: Node? = null
        private set
    var isRunning: Boolean = false
        private set
    var nodeId: String = ""
        private set
    var channels: List<ChannelDetails> = emptyList()
        private set
    var savedMnemonic: String? = run {
        // Pre-load saved mnemonic from disk so it's available immediately
        val file = File(Constants.userDataDir(context), "seed_phrase")
        if (file.exists()) file.readText().trim().ifEmpty { null } else null
    }
        private set

    private var eventJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO)

    private val _events = MutableSharedFlow<Event>(extraBufferCapacity = 64)
    val events: SharedFlow<Event> = _events

    fun start(network: Network, esploraURL: String, mnemonic: String?) {
        val dataDir = Constants.userDataDir(context)

        val anchorConfig = AnchorChannelsConfig(
            trustedPeersNoReserve = listOf(Constants.DEFAULT_LSP_PUBKEY),
            perChannelReserveSats = 25_000UL
        )

        val config = Config(
            storageDirPath = dataDir.absolutePath,
            network = network,
            listeningAddresses = null,
            announcementAddresses = null,
            nodeAlias = null,
            trustedPeers0conf = listOf(Constants.DEFAULT_LSP_PUBKEY),
            probingLiquidityLimitMultiplier = 3UL,
            anchorChannelsConfig = anchorConfig,
            routeParameters = null
        )

        val builder = Builder.fromConfig(config)
        builder.setChainSourceEsplora(esploraURL, null)

        val rgsUrl = when (network) {
            Network.BITCOIN -> Constants.RGSServer.BITCOIN
            Network.SIGNET -> Constants.RGSServer.SIGNET
            Network.TESTNET -> Constants.RGSServer.TESTNET
            else -> Constants.RGSServer.BITCOIN
        }
        builder.setGossipSourceRgs(rgsUrl)

        builder.setLiquiditySourceLsps2(
            Constants.DEFAULT_LSP_PUBKEY,
            Constants.DEFAULT_LSP_ADDRESS,
            null
        )

        val seedPhrasePath = File(Constants.userDataDir(context), "seed_phrase")
        val keySeedPath = File(Constants.userDataDir(context), "keys_seed")

        // Determine which mnemonic to use
        val words: String = if (mnemonic != null) {
            // Restore — wipe ALL wallet data so new seed takes effect
            wipeWalletData(context)
            mnemonic.trim()
        } else if (seedPhrasePath.exists()) {
            // Existing wallet — re-read saved mnemonic
            seedPhrasePath.readText().trim()
        } else if (!keySeedPath.exists()) {
            // Truly new wallet — no seed_phrase, no keys_seed
            wipeWalletData(context)
            generateEntropyMnemonic(null)
        } else {
            // Pre-upgrade wallet with only keys_seed, no mnemonic available
            ""
        }

        // Save mnemonic to file and set on builder
        if (words.isNotEmpty()) {
            seedPhrasePath.writeText(words)
            savedMnemonic = words
            builder.setEntropyBip39Mnemonic(words, null)
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
        n.spliceOut(userChannelId, counterpartyNodeId, address, amountSats.toULong())
    }

    fun sendPayment(invoice: Bolt11Invoice): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt11Payment().send(invoice, null)
    }

    fun sendPaymentUsingAmount(invoice: Bolt11Invoice, amountMsat: Long): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt11Payment().sendUsingAmount(invoice, amountMsat.toULong(), null)
    }

    fun sendBolt12(offer: Offer): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt12Payment().send(offer, null, null, null)
    }

    fun sendBolt12UsingAmount(offer: Offer, amountMsat: Long): String {
        val n = node ?: throw NodeServiceError()
        return n.bolt12Payment().sendUsingAmount(offer, amountMsat.toULong(), null, null, null)
    }

    fun sendKeysend(amountMsat: Long, toNodeId: String): String {
        val n = node ?: throw NodeServiceError()
        return n.spontaneousPayment().send(amountMsat.toULong(), toNodeId, null)
    }

    fun sendKeysendWithTLV(amountMsat: Long, toNodeId: String, tlvs: List<CustomTlvRecord>): String {
        val n = node ?: throw NodeServiceError()
        return n.spontaneousPayment().sendWithCustomTlvs(amountMsat.toULong(), toNodeId, null, tlvs)
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
        return n.onchainPayment().sendAllToAddress(address, false, null)
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
        return n.signMessage(message.map { it.toUByte() })
    }

    fun verifySignature(message: ByteArray, signature: String, pubkey: String): Boolean {
        return try {
            val n = node ?: return false
            n.verifySignature(message.map { it.toUByte() }, signature, pubkey)
        } catch (_: Exception) { false }
    }

    companion object {
        fun wipeWalletData(context: Context) {
            val dir = Constants.userDataDir(context)
            listOf(
                "keys_seed",
                "seed_phrase",
                "ldk_node_data.sqlite",
                "ldk_node_data.sqlite-wal",
                "ldk_node_data.sqlite-shm",
            ).forEach { File(dir, it).delete() }
        }
    }

    class NodeServiceError : Exception("Node not running")
}
