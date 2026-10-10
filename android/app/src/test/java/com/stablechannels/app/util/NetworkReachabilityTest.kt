package com.stablechannels.app.util

import java.io.IOException
import java.io.InterruptedIOException
import java.net.ConnectException
import java.net.NoRouteToHostException
import java.net.PortUnreachableException
import java.net.ProtocolException
import java.net.SocketException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import javax.net.ssl.SSLException
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.lightningdevkit.ldknode.NodeException

class NetworkReachabilityTest {

    @Test
    fun `isNetworkError identifies socket and connection exceptions`() {
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(UnknownHostException("mempool.space"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(SocketTimeoutException("Read timed out"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(ConnectException("Connection refused"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(NoRouteToHostException("No route to host"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                PortUnreachableException("Port unreachable")
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(SocketException("Network is unreachable"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(SSLException("SSL handshake failed"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(ProtocolException("Connection reset"))
        )
        assertTrue(NetworkReachabilityEvaluator.isNetworkError(InterruptedIOException("Timeout")))
    }

    @Test
    fun `isNetworkError identifies typed NodeException variants`() {
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(NodeException.ConnectionFailed("network"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                NodeException.LiquiditySourceUnavailable("network")
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                NodeException.FeerateEstimationUpdateFailed("network")
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                NodeException.FeerateEstimationUpdateTimeout("network")
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(NodeException.TxSyncFailed("network"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(NodeException.TxSyncTimeout("network"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(NodeException.GossipUpdateFailed("network"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                NodeException.GossipUpdateTimeout("network")
            )
        )
    }

    @Test
    fun `isNetworkError identifies wrapped cause exceptions`() {
        val wrapped = RuntimeException("Outer failure", ConnectException("Connection refused"))
        assertTrue(NetworkReachabilityEvaluator.isNetworkError(wrapped))

        val deeplyWrapped =
            Exception("Level 1", IOException("Level 2", UnknownHostException("api.host")))
        assertTrue(NetworkReachabilityEvaluator.isNetworkError(deeplyWrapped))
    }

    @Test
    fun `isNetworkError identifies network error message substrings`() {
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(Exception("Client is currently offline"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                Exception("The internet connection appears to be offline")
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(Exception("network connection was lost"))
        )
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(
                Exception("Failed to connect to /127.0.0.1:9735")
            )
        )
        assertTrue(NetworkReachabilityEvaluator.isNetworkError(Exception("cannot connect to host")))
        assertTrue(
            NetworkReachabilityEvaluator.isNetworkError(Exception("connection refused by peer"))
        )
    }

    @Test
    fun `isNetworkError returns false for non-network errors`() {
        assertFalse(NetworkReachabilityEvaluator.isNetworkError(null))
        assertFalse(
            NetworkReachabilityEvaluator.isNetworkError(IllegalArgumentException("Invalid invoice"))
        )
        assertFalse(
            NetworkReachabilityEvaluator.isNetworkError(IllegalStateException("Database not open"))
        )
        assertFalse(
            NetworkReachabilityEvaluator.isNetworkError(NullPointerException("Missing parameter"))
        )
    }

    @Test
    fun `shouldPresentOfflineNotice returns true when network is offline regardless of error`() {
        assertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(null, isNetworkOffline = true)
        )
        assertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(
                IllegalArgumentException("any"),
                isNetworkOffline = true,
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(
                ConnectException("failed"),
                isNetworkOffline = true,
            )
        )
    }

    @Test
    fun `shouldPresentOfflineNotice delegates to isNetworkError when network is online`() {
        assertFalse(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(null, isNetworkOffline = false)
        )
        assertFalse(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(
                IllegalArgumentException("any"),
                isNetworkOffline = false,
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(
                ConnectException("Connection refused"),
                isNetworkOffline = false,
            )
        )
        assertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(
                UnknownHostException("cannot resolve"),
                isNetworkOffline = false,
            )
        )
    }
}
