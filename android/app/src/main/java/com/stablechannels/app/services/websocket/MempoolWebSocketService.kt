package com.stablechannels.app.services.websocket

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class MempoolWebSocketService(
    private val endpointUrl: String = "wss://mempool.space/api/v1/ws",
    private val serviceScope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO),
    private val dedupStore: ProcessedTxStore = ProcessedTxStore(),
    private val matcher: TransactionMatcher = TransactionMatcher(),
    private val client: OkHttpClient = OkHttpClient.Builder()
        .pingInterval(30, TimeUnit.SECONDS)
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .build(),
    private val connectionFactory: WebSocketConnectionFactory = OkHttpWebSocketConnectionFactory(client)
) : MempoolWebSocketClient {

    companion object {
        private const val TAG = "MempoolWebSocket"
    }

    private val json = Json { ignoreUnknownKeys = true }
    private val reconnectManager = ReconnectionManager(serviceScope)

    private val trackedAddresses = linkedSetOf<String>()
    private val trackedTxids = linkedSetOf<String>()
    private val pendingOutboundMessages = ArrayDeque<String>()

    private val socketConnected = AtomicBoolean(false)
    private val socketConnecting = AtomicBoolean(false)

    private var webSocket: WebSocketConnection? = null

    override val isConnected: Boolean
        get() = socketConnected.get()

    override var onTransactionDetected: ((WebSocketEvent) -> Unit)? = null
    override var onBlockHeader: ((Int) -> Unit)? = null

    init {
        reconnectManager.onReconnect = {
            connect()
        }
    }

    // Avoid test-runtime crashes when android.util.Log is not mocked in JVM unit tests.
    private fun logInfo(message: String) {
        try {
            Log.i(TAG, message)
        } catch (_: Throwable) {
        }
    }

    private fun logWarn(message: String) {
        try {
            Log.w(TAG, message)
        } catch (_: Throwable) {
        }
    }

    override fun connect() {
        if (socketConnected.get() || socketConnecting.get()) {
            return
        }

        reconnectManager.reset()
        reconnectManager.stopReconnectTask()
        socketConnecting.set(true)

        webSocket?.cancel()
        webSocket = null

        webSocket = connectionFactory.create(
            endpointUrl,
            WebSocketCallbacks(
                onOpen = {
                    socketConnecting.set(false)
                    socketConnected.set(true)
                    reconnectManager.connected()
                    logInfo("WebSocket connected")

                    syncTracking()
                    subscribeToBlocks()
                    flushPendingMessages()
                },
                onMessage = { text ->
                    handleMessage(text)
                },
                onClosing = { code, reason ->
                    // Do not echo peer close codes; some reserved codes (e.g. 1005) are invalid to send.
                    handleDisconnection("onClosing: $code $reason")
                },
                onClosed = { code, reason ->
                    handleDisconnection("onClosed: $code $reason")
                },
                onFailure = { throwable ->
                    handleDisconnection("onFailure: ${throwable.message}")
                }
            )
        )
        logInfo("Connecting to $endpointUrl")
    }

    override fun disconnect() {
        reconnectManager.stop()
        socketConnecting.set(false)
        socketConnected.set(false)
        webSocket?.close(1000, "manual disconnect")
        webSocket = null
        logInfo("Disconnected")
    }

    override fun trackAddress(address: String) {
        val normalized = address.trim()
        if (normalized.isEmpty()) {
            return
        }
        synchronized(trackedAddresses) {
            trackedAddresses.add(normalized)
        }

        if (isConnected) {
            syncTracking()
        } else {
            connect()
        }
    }

    override fun untrackAddress(address: String) {
        val normalized = address.trim()
        if (normalized.isEmpty()) {
            return
        }
        synchronized(trackedAddresses) {
            trackedAddresses.remove(normalized)
        }
        if (isConnected) {
            syncTracking()
        }
    }

    override fun trackTx(txid: String) {
        val normalized = txid.trim()
        if (!isValidTxid(normalized)) {
            return
        }
        synchronized(trackedTxids) {
            trackedTxids.add(normalized)
        }

        if (isConnected) {
            syncTracking()
        } else {
            connect()
        }
    }

    override fun untrackTx(txid: String) {
        val normalized = txid.trim()
        if (normalized.isEmpty()) {
            return
        }
        synchronized(trackedTxids) {
            trackedTxids.remove(normalized)
        }
        if (isConnected) {
            syncTracking()
        }
    }

    private fun handleDisconnection(message: String) {
        if (!socketConnected.get() && !socketConnecting.get()) {
            return
        }
        socketConnecting.set(false)
        socketConnected.set(false)
        webSocket = null
        logWarn("Disconnected: $message")
        reconnectManager.disconnected()
    }

    private fun subscribeToBlocks() {
        send("""{ "action": "want", "data": ["blocks", "mempool-blocks"] }""")
    }

    private fun send(text: String) {
        val socket = webSocket
        if (!isConnected || socket == null) {
            synchronized(pendingOutboundMessages) {
                pendingOutboundMessages.addLast(text)
                if (pendingOutboundMessages.size > 50) {
                    pendingOutboundMessages.removeFirst()
                }
            }
            return
        }
        socket.send(text)
    }

    private fun flushPendingMessages() {
        val queued = mutableListOf<String>()
        synchronized(pendingOutboundMessages) {
            while (pendingOutboundMessages.isNotEmpty()) {
                queued.add(pendingOutboundMessages.removeFirst())
            }
        }
        queued.forEach(::send)
    }

    private fun syncTracking() {
        val addresses = synchronized(trackedAddresses) { trackedAddresses.toList() }
        val txids = synchronized(trackedTxids) { trackedTxids.toList() }

        send(json.encodeToString(mapOf("track-addresses" to addresses)))
        send(json.encodeToString(mapOf("track-txs" to txids)))
    }

    private fun handleMessage(text: String) {
        val msg = try {
            json.decodeFromString<MempoolWSMessage>(text)
        } catch (_: Exception) {
            logWarn("Failed to decode message: ${text.take(200)}")
            return
        }

        val trackedAddressesSnapshot = synchronized(trackedAddresses) { trackedAddresses.toSet() }
        val trackedTxidsSnapshot = synchronized(trackedTxids) { trackedTxids.toSet() }

        val transactions = aggregateTransactions(msg)
        transactions.forEach { tx ->
            if (!isValidTxid(tx.txid)) {
                return@forEach
            }
            val matches = matcher.matchAll(
                trackedAddresses = trackedAddressesSnapshot,
                trackedTxids = trackedTxidsSnapshot,
                msg = msg,
                tx = tx
            )
            // isTxid matches (spends of a tracked txid) are handled by the dedicated
            // tracked-txs/utxoSpent branch below via TrackedOutspend; only address
            // matches represent an actual incoming receive.
            matches.filterNot { it.isTxid }.forEach { match ->
                val dedupKey = "${tx.txid}_${match.target}"
                if (dedupStore.isRecentlyProcessed(dedupKey)) {
                    return@forEach
                }
                dedupStore.recordProcessedTx(dedupKey)

                val amountSats = sumAmount(tx, match.target)
                notifyTransaction(WebSocketEvent.Receive(match.target, tx.txid, amountSats))
            }
        }

        handleRemovedTransactions(msg, trackedAddressesSnapshot)

        val blockHeight = msg.block?.height ?: msg.blocks?.lastOrNull()?.height
        if (blockHeight != null) {
            notifyBlock(blockHeight)
        }

        val tracked = msg.trackedTxs ?: emptyMap()
        tracked.forEach { (trackedTxid, trackingInfo) ->
            if (!trackedTxidsSnapshot.contains(trackedTxid)) {
                return@forEach
            }
            val outspends = trackingInfo.utxoSpent ?: emptyMap()
            outspends.forEach { (_, outspend) ->
                val spendingTxid = outspend.txid
                if (!isValidTxid(spendingTxid)) {
                    return@forEach
                }
                val dedupKey = "${spendingTxid}_outspend_$trackedTxid"
                if (dedupStore.isRecentlyProcessed(dedupKey)) {
                    return@forEach
                }
                dedupStore.recordProcessedTx(dedupKey)
                notifyTransaction(WebSocketEvent.TrackedOutspend(trackedTxid, spendingTxid))
            }
        }
    }

    private fun aggregateTransactions(msg: MempoolWSMessage): List<MempoolWSTransaction> {
        val all = mutableListOf<MempoolWSTransaction>()
        all.addAll(msg.addressTransactions ?: emptyList())
        all.addAll(msg.blockTransactions ?: emptyList())

        msg.multiAddressTransactions?.forEach { (_, txGroup) ->
            all.addAll(txGroup.mempool ?: emptyList())
            all.addAll(txGroup.confirmed ?: emptyList())
        }

        return all
    }

    private fun handleRemovedTransactions(
        msg: MempoolWSMessage,
        trackedAddressesSnapshot: Set<String>
    ) {
        msg.multiAddressTransactions?.forEach { (addr, txGroup) ->
            if (!trackedAddressesSnapshot.contains(addr)) {
                return@forEach
            }
            (txGroup.removed ?: emptyList()).forEach { tx ->
                if (!isValidTxid(tx.txid)) {
                    return@forEach
                }
                val dedupKey = "removed_${tx.txid}_$addr"
                if (dedupStore.isRecentlyProcessed(dedupKey)) {
                    return@forEach
                }
                dedupStore.recordProcessedTx(dedupKey)
                notifyTransaction(WebSocketEvent.Removed(addr, tx.txid))
            }
        }
    }

    private fun sumAmount(tx: MempoolWSTransaction, target: String): Long {
        var amountSats = 0L
        tx.vout?.forEach { vout ->
            if (vout.scriptpubkeyAddress == target) {
                amountSats += vout.value ?: 0L
            }
        }
        return amountSats
    }

    private fun notifyTransaction(event: WebSocketEvent) {
        val callback = onTransactionDetected ?: return
        serviceScope.launch(Dispatchers.Main.immediate) {
            callback(event)
        }
    }

    private fun notifyBlock(height: Int) {
        val callback = onBlockHeader ?: return
        serviceScope.launch(Dispatchers.Main.immediate) {
            callback(height)
        }
    }

    private fun isValidTxid(txid: String): Boolean {
        return txid.length == 64 && txid.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
    }
}
