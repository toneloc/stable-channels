package com.stablechannels.app

import com.stablechannels.app.domain.EmailAddress
import com.stablechannels.app.services.EmailPromptStore
import com.stablechannels.app.services.LdkNodeOwner
import com.stablechannels.app.services.NodeService
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ToRemoteRecoveryRestoreTest {
    private fun nodeField(name: String) =
        NodeService::class.java.getDeclaredField(name).apply { isAccessible = true }

    @Suppress("UNCHECKED_CAST")
    private fun markNodeRunning(state: AppState) {
        (nodeField("_isRunning").get(state.nodeService) as MutableStateFlow<Boolean>).value = true
    }

    private fun toRemoteKeys(state: AppState): Any? =
        AppState::class
            .java
            .getDeclaredMethod("toRemoteKeys")
            .apply { isAccessible = true }
            .invoke(state)

    /**
     * A restore stops the node, resets the recovery, then starts the node with the restored seed. A
     * recovery run in between gets no keys, not those of the wallet still in memory (#364).
     */
    @Suppress("UNCHECKED_CAST")
    @Test
    fun noKeysWhileTheNodeIsStopped() {
        val state = AppState(RuntimeEnvironment.getApplication())
        (nodeField("_savedMnemonic").get(state.nodeService) as MutableStateFlow<String?>).value =
            "abandon ".repeat(11) + "about"
        assertNull(toRemoteKeys(state))

        markNodeRunning(state)
        assertNotNull(toRemoteKeys(state))
    }

    /**
     * The restored node start wipes the old wallet files before it can fail, so the old wallet's
     * email and prompt answers are forgotten before the start, not after it.
     */
    @Test
    fun aFailedRestoreStillForgetsTheOldEmail() {
        val context = RuntimeEnvironment.getApplication()
        val state = AppState(context)
        val store = EmailPromptStore.get(context)
        store.savedEmail = EmailAddress.parse("old@example.com")
        // Another owner holds the node data, so the start throws.
        assertTrue(LdkNodeOwner.tryAcquire(LdkNodeOwner.STABILITY_SERVICE))
        try {
            runBlocking {
                runCatching { state.startRestoredNode("abandon ".repeat(11) + "about", false) }
            }
        } finally {
            LdkNodeOwner.release(LdkNodeOwner.STABILITY_SERVICE)
        }

        assertNull(store.savedEmail)
    }
}
