package com.stablechannels.app.services.websocket

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.min
import kotlin.math.pow

class ReconnectionManager(
    private val scope: CoroutineScope,
    private val maxReconnectDelaySeconds: Long = 60L,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO
) {
    @Volatile
    var reconnectAttempts: Int = 0
        private set

    @Volatile
    var isManualDisconnect: Boolean = false

    private var reconnectJob: Job? = null

    var onReconnect: (() -> Unit)? = null

    fun connected() {
        reconnectAttempts = 0
        stopReconnectTask()
    }

    fun disconnected() {
        if (isManualDisconnect) {
            return
        }
        scheduleReconnect()
    }

    fun stop() {
        isManualDisconnect = true
        stopReconnectTask()
    }

    fun reset() {
        isManualDisconnect = false
        reconnectAttempts = 0
    }

    private fun scheduleReconnect() {
        stopReconnectTask()

        val delaySeconds = min((2.0.pow(reconnectAttempts.toDouble())).toLong(), maxReconnectDelaySeconds)
        reconnectAttempts += 1

        reconnectJob = scope.launch(dispatcher) {
            delay(delaySeconds * 1000)
            if (!isManualDisconnect) {
                onReconnect?.invoke()
            }
        }
    }

    fun stopReconnectTask() {
        reconnectJob?.cancel()
        reconnectJob = null
    }
}
