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
    val cm = getSystemService(ConnectivityManager::class.java) ?: return false
    val network = cm.activeNetwork ?: return false
    val caps = cm.getNetworkCapabilities(network) ?: return false
    return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
}

/** Emits online/offline changes for the default network. */
fun Context.observeOnline(): Flow<Boolean> = callbackFlow {
    val cm = getSystemService(ConnectivityManager::class.java)
    val callback =
        object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(
                network: Network,
                caps: NetworkCapabilities,
            ) {
                val hasInternet = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                trySend(hasInternet)
            }

            override fun onAvailable(network: Network) {
                trySend(isOnline())
            }

            override fun onLost(network: Network) {
                val active = cm?.activeNetwork
                if (active == null || active == network) {
                    trySend(false)
                } else {
                    trySend(isOnline())
                }
            }

            override fun onUnavailable() {
                trySend(false)
            }
        }
    trySend(isOnline())
    cm?.registerDefaultNetworkCallback(callback)
    awaitClose { cm?.unregisterNetworkCallback(callback) }
}
    .distinctUntilChanged()

/** Evaluates network connectivity conditions and error classification. */
object NetworkReachabilityEvaluator {
    fun isNetworkError(throwable: Throwable?): Boolean {
        if (throwable == null) return false
        var curr: Throwable? = throwable
        while (curr != null) {
            if (
                curr is java.net.UnknownHostException ||
                    curr is java.net.SocketTimeoutException ||
                    curr is java.net.ConnectException ||
                    curr is java.net.NoRouteToHostException ||
                    curr is java.net.PortUnreachableException ||
                    curr is java.net.SocketException ||
                    curr is javax.net.ssl.SSLException
            ) {
                return true
            }
            val msg = curr.message?.lowercase() ?: ""
            if (
                msg.contains("offline") ||
                    msg.contains("not connected") ||
                    msg.contains("network connection was lost") ||
                    msg.contains("cannot connect") ||
                    msg.contains("failed to connect") ||
                    msg.contains("timed out") ||
                    msg.contains("timeout") ||
                    msg.contains("unreachable") ||
                    msg.contains("connection refused")
            ) {
                return true
            }
            curr = curr.cause
        }
        return false
    }

    fun shouldPresentOfflineNotice(throwable: Throwable?, isNetworkOffline: Boolean): Boolean {
        if (isNetworkOffline) return true
        return isNetworkError(throwable)
    }
}

object OfflineMessages {
    const val TITLE = "No Internet Connection"
    const val BODY =
        "Cannot connect to the server without an internet connection. Your wallet and keys remain safe on this device."
    const val SEND =
        "You're offline. Payments cannot be sent until network connectivity is restored."
    const val TRADE =
        "You're offline. Trades cannot be executed until network connectivity is restored."
    const val CLOSE_CHANNEL = "Reconnect to close your channel"
    const val CHECK_NETWORK = "Please check your network connection"
    const val HOME_INFO = "Please check your network connection"
    const val LSP_INFO = "Reconnect to switch your LSP"
}
