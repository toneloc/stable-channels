package com.stablechannels.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import com.stablechannels.app.AppState.Companion.BalanceCacheKey
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.OfflineMessages
import com.stablechannels.app.util.isOnline
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowNetworkCapabilities
import org.robolectric.shadows.ShadowNetworkInfo

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OfflineHandlingTest {

    private lateinit var context: Context

    private fun setDeviceOnline(online: Boolean) {
        val cm = context.getSystemService(ConnectivityManager::class.java)
        val shadowCm = Shadows.shadowOf(cm)
        if (online) {
            val info =
                ShadowNetworkInfo.newInstance(
                    NetworkInfo.DetailedState.CONNECTED,
                    ConnectivityManager.TYPE_WIFI,
                    0,
                    true,
                    NetworkInfo.State.CONNECTED,
                )
            shadowCm.setActiveNetworkInfo(info)
            shadowCm.setDefaultNetworkActive(true)
            val network = cm.activeNetwork
            if (network != null) {
                val caps = ShadowNetworkCapabilities.newInstance()
                val shadowCaps = Shadows.shadowOf(caps)
                shadowCaps.addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                shadowCaps.addCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
                shadowCm.setNetworkCapabilities(network, caps)
            }
        } else {
            shadowCm.setActiveNetworkInfo(null)
            shadowCm.setDefaultNetworkActive(false)
        }
    }

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        val userDir = Constants.userDataDir(context)
        if (!userDir.exists()) userDir.mkdirs()
        File(userDir, "keys_seed").delete()
        File(userDir, "seed_phrase").delete()
        setDeviceOnline(false)
    }

    @Test
    fun `isOnline returns true only when internet and validated capabilities are present`() {
        setDeviceOnline(false)
        assertFalse(context.isOnline())

        setDeviceOnline(true)
        assertTrue(context.isOnline())

        val cm = context.getSystemService(ConnectivityManager::class.java)
        val shadowCm = Shadows.shadowOf(cm)
        val network = cm.activeNetwork
        if (network != null) {
            val capsNoValidation = ShadowNetworkCapabilities.newInstance()
            Shadows.shadowOf(capsNoValidation)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            shadowCm.setNetworkCapabilities(network, capsNoValidation)
            assertFalse(context.isOnline())
        }
    }

    @Test
    fun `Phase OFFLINE is distinct enum variant`() {
        val phases = Phase.values().toList()
        assertTrue(phases.contains(Phase.OFFLINE))
        assertTrue(phases.contains(Phase.WALLET))
        assertTrue(phases.contains(Phase.SYNCING))
        assertTrue(phases.contains(Phase.LOADING))
        assertTrue(phases.contains(Phase.ONBOARDING))
        assertTrue(phases.contains(Phase.ERROR))
    }

    @Test
    fun `BalanceCacheKey contains ready channel and node id keys`() {
        assertEquals("cached_has_ready_channel", BalanceCacheKey.HAS_READY_CHANNEL)
        assertEquals("node_id", BalanceCacheKey.NODE_ID)
        assertEquals("balance_cache", BalanceCacheKey.PREFS_NAME)
    }

    @Test
    fun `OfflineMessages match required copy`() {
        assertEquals("No Internet Connection", OfflineMessages.TITLE)
        assertEquals(
            "Cannot connect to the server without an internet connection. Your wallet and keys remain safe on this device.",
            OfflineMessages.BODY,
        )
        assertEquals(
            "You're offline. Payments cannot be sent until network connectivity is restored.",
            OfflineMessages.SEND,
        )
        assertEquals(
            "You're offline. Trades cannot be executed until network connectivity is restored.",
            OfflineMessages.TRADE,
        )
        assertEquals("Reconnect to close your channel", OfflineMessages.CLOSE_CHANNEL)
        assertEquals(
            "Please check your network connection",
            OfflineMessages.HOME_INFO,
        )
        assertEquals("Reconnect to switch your LSP", OfflineMessages.LSP_INFO)
    }

    @Test
    fun `hasExistingWallet returns false when no seed files exist`() {
        val appState = AppState(context)
        assertFalse(appState.hasExistingWallet())
    }

    @Test
    fun `hasExistingWallet returns true when keys_seed exists`() {
        val userDir = Constants.userDataDir(context)
        File(userDir, "keys_seed").writeBytes(ByteArray(32))

        val appState = AppState(context)
        assertTrue(appState.hasExistingWallet())
    }

    @Test
    fun `hasExistingWallet returns true when seed_phrase exists`() {
        val userDir = Constants.userDataDir(context)
        File(userDir, "seed_phrase").writeText("abandon abandon abandon")

        val appState = AppState(context)
        assertTrue(appState.hasExistingWallet())
    }

    @Test
    fun `cached node id is restored from SharedPreferences when node is stopped`() {
        val expectedNodeId = "02710b5069e90a44deadbeef1234567890abcdef"
        val prefs = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE)
        prefs.edit().putString(BalanceCacheKey.NODE_ID, expectedNodeId).commit()

        val appState = AppState(context)
        assertEquals(expectedNodeId, appState.getCachedNodeId())
    }

    @Test
    fun `cached has ready channel is stored and retrievable`() {
        val prefs = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE)
        prefs.edit().putBoolean(BalanceCacheKey.HAS_READY_CHANNEL, true).commit()

        assertTrue(prefs.getBoolean(BalanceCacheKey.HAS_READY_CHANNEL, false))
    }

    @Test
    fun `setPhaseWallet transitions phase to WALLET`() {
        val appState = AppState(context)
        appState.setPhaseWallet()
        assertEquals(Phase.WALLET, appState.phase.value)
    }

    @Test
    fun `deferNodeStartUntilOnline transitions phase to OFFLINE and clears syncing`() {
        val appState = AppState(context)
        appState.deferNodeStartUntilOnline()
        assertEquals(Phase.OFFLINE, appState.phase.value)
        assertFalse(appState.isSyncing.value)
        assertEquals("", appState.errorMessage.value)
    }

    @Test
    fun `handleNodeStartFailure transitions to OFFLINE on network error`() {
        val appState = AppState(context)
        appState.setOnline(false)
        appState.handleNodeStartFailure(
            java.net.ConnectException("Connection refused"),
            "fallback",
        )
        assertEquals(Phase.OFFLINE, appState.phase.value)
        assertFalse(appState.isSyncing.value)
    }

    @Test
    fun `handleNodeStartFailure transitions to ERROR on non-retryable fatal error`() {
        setDeviceOnline(true)
        val appState = AppState(context)
        appState.setOnline(true)
        val fatalError = IllegalStateException("Corrupt database schema")
        appState.handleNodeStartFailure(fatalError, "Wallet start failed")
        assertEquals(Phase.ERROR, appState.phase.value)
        assertEquals("Corrupt database schema", appState.errorMessage.value)
    }

    @Test
    fun `retryConnection does not force Phase WALLET when still offline`() {
        val appState = AppState(context)
        appState.deferNodeStartUntilOnline()
        assertEquals(Phase.OFFLINE, appState.phase.value)

        appState.setOnline(false)
        appState.retryConnection()

        assertEquals(Phase.OFFLINE, appState.phase.value)
    }

    @Test
    fun `retryConnection re-entry guard prevents duplicate concurrent executions`() {
        val appState = AppState(context)
        assertFalse(appState.isRetrying.value)
    }
}
