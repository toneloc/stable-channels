package com.stablechannels.app.services

import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.Network
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

private val VALID = "abandon ".repeat(11) + "about"
private val BAD_CHECKSUM = "abandon ".repeat(11) + "abandon"

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class NodeServiceRestoreTest {
    private val context = RuntimeEnvironment.getApplication()
    private val dir = Constants.userDataDir(context)

    /** A phrase that fails BIP39 must not wipe or overwrite the current wallet. */
    @Test
    fun `restoring an invalid phrase fails before touching wallet files`() {
        val files = listOf("seed_phrase", "keys_seed", "ldk_node_data.sqlite")
        files.forEach { File(dir, it).writeText("old $it") }

        assertThrows(IllegalArgumentException::class.java) {
            NodeService(context).start(Network.BITCOIN, "https://example.invalid", BAD_CHECKSUM)
        }

        files.forEach { assertEquals("old $it", File(dir, it).readText()) }
    }

    @Test
    fun `only phrases with a valid BIP39 checksum and words pass`() {
        assertTrue(NodeService.isValidMnemonic(VALID))
        assertTrue(NodeService.isValidMnemonic("  ${VALID.replace(" ", "\n ")} "))
        assertFalse(NodeService.isValidMnemonic(BAD_CHECKSUM))
        assertFalse(NodeService.isValidMnemonic(VALID.replace("about", "abuot")))
        assertFalse(NodeService.isValidMnemonic(""))
    }
}
