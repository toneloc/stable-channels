package com.stablechannels.app.services.websocket

sealed class WebSocketEvent {
    data class Receive(val target: String, val txid: String, val amountSats: Long) : WebSocketEvent()
    data class Removed(val target: String, val txid: String) : WebSocketEvent()
    data class TrackedOutspend(val trackedTxid: String, val spendingTxid: String) : WebSocketEvent()
}

interface MempoolWebSocketClient {
    val isConnected: Boolean
    var onTransactionDetected: ((WebSocketEvent) -> Unit)?
    var onBlockHeader: ((Int) -> Unit)?

    fun connect()
    fun disconnect()
    fun trackAddress(address: String)
    fun untrackAddress(address: String)
    fun trackTx(txid: String)
    fun untrackTx(txid: String)
}
