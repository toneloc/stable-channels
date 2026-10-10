package com.stablechannels.app.util

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.distinctUntilChanged

/** True if the active network claims internet access. */
fun Context.isOnline(): Boolean {
    val cm = getSystemService(ConnectivityManager::class.java)
    val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
    return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
}

/** Emits online/offline changes for the default network. */
fun Context.observeOnline(): Flow<Boolean> = callbackFlow {
    val cm = getSystemService(ConnectivityManager::class.java)
    val callback =
        object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                trySend(isOnline())
            }

            override fun onLost(network: Network) {
                trySend(isOnline())
            }
        }
    trySend(isOnline())
    cm.registerDefaultNetworkCallback(callback)
    awaitClose { cm.unregisterNetworkCallback(callback) }
}
    .distinctUntilChanged()

object OfflineMessages {
    const val SEND =
        "You're offline. Payments cannot be sent until network connectivity is restored."
    const val TRADE =
        "You're offline. Trades cannot be executed until network connectivity is restored."
    const val CLOSE_CHANNEL = "Reconnect to close your channel."
    const val HOME_INFO = "Sending and receiving resume when you're back online."
    const val LSP_INFO = "Reconnect to switch your LSP."
}
