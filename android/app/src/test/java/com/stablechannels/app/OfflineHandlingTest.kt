package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.AppState.Companion.BalanceCacheKey
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.OfflineMessages
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class OfflineHandlingTest {

    private lateinit var context: Context

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        val userDir = Constants.userDataDir(context)
        if (!userDir.exists()) userDir.mkdirs()
        File(userDir, "keys_seed").delete()
        File(userDir, "seed_phrase").delete()
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
}
