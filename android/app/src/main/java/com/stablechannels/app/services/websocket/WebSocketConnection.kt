package com.stablechannels.app.services.websocket

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener

interface WebSocketConnection {
    fun send(text: String): Boolean
    fun close(code: Int, reason: String?): Boolean
    fun cancel()
}

data class WebSocketCallbacks(
    val onOpen: () -> Unit,
    val onMessage: (text: String) -> Unit,
    val onClosing: (code: Int, reason: String) -> Unit,
    val onClosed: (code: Int, reason: String) -> Unit,
    val onFailure: (Throwable) -> Unit
)

interface WebSocketConnectionFactory {
    fun create(endpointUrl: String, callbacks: WebSocketCallbacks): WebSocketConnection
}

class OkHttpWebSocketConnectionFactory(
    private val client: OkHttpClient
) : WebSocketConnectionFactory {
    override fun create(endpointUrl: String, callbacks: WebSocketCallbacks): WebSocketConnection {
        val request = Request.Builder().url(endpointUrl).build()
        val webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                callbacks.onOpen()
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                callbacks.onMessage(text)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                callbacks.onClosing(code, reason)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                callbacks.onClosed(code, reason)
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                callbacks.onFailure(t)
            }
        })

        return object : WebSocketConnection {
            override fun send(text: String): Boolean = webSocket.send(text)

            override fun close(code: Int, reason: String?): Boolean = webSocket.close(code, reason)

            override fun cancel() {
                webSocket.cancel()
            }
        }
    }
}