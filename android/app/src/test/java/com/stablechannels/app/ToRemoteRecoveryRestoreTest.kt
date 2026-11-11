package com.stablechannels.app

import com.stablechannels.app.services.NodeService
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
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
    @Test
    fun noKeysWhileTheNodeIsStopped() {
        val state = AppState(RuntimeEnvironment.getApplication())
        nodeField("savedMnemonic").set(state.nodeService, "abandon ".repeat(11) + "about")
        assertNull(toRemoteKeys(state))

        markNodeRunning(state)
        assertNotNull(toRemoteKeys(state))
    }
}
