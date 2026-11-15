package com.stablechannels.app.services

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.lightningdevkit.ldknode.ChannelShutdownState

class EmergencyCloseTest {
    private val calls = mutableListOf<String>()
    private val refused = mutableListOf<Exception>()

    private fun closeChannel(force: Boolean, closeError: Exception? = null): Boolean =
        EmergencyClose.closeChannel(
            force,
            close = {
                calls += "close"
                closeError?.let { throw it }
            },
            forceClose = { calls += "force close" },
            onRefused = { refused += it },
        )

    @Test
    fun triesANormalCloseOnlyWhileTheLspIsOnlineAndNoCloseIsUnderWay() {
        val notShuttingDown = ChannelShutdownState.NOT_SHUTTING_DOWN
        assertFalse(EmergencyClose.forcesAtOnce(false, true, notShuttingDown))
        assertFalse(EmergencyClose.forcesAtOnce(false, true, null))
        assertTrue(EmergencyClose.forcesAtOnce(true, true, notShuttingDown))
        assertTrue(EmergencyClose.forcesAtOnce(false, false, notShuttingDown))
        // A close under way, after a restart or started by the LSP, which this session never saw.
        for (state in
            listOf(
                ChannelShutdownState.SHUTDOWN_INITIATED,
                ChannelShutdownState.RESOLVING_HTL_CS,
                ChannelShutdownState.NEGOTIATING_CLOSING_FEE,
            )) {
            assertTrue(EmergencyClose.forcesAtOnce(false, true, state))
        }
    }

    @Test
    fun keepsANormalCloseThatLdkTakes() {
        assertFalse(closeChannel(force = false))

        assertEquals(listOf("close"), calls)
    }

    @Test
    fun forcesANormalCloseThatLdkRefuses() {
        val error = IllegalStateException("Shutdown already in progress")

        assertTrue(closeChannel(force = false, closeError = error))

        assertEquals(listOf("close", "force close"), calls)
        assertEquals(listOf<Exception>(error), refused)
    }

    @Test
    fun forcesWithoutANormalCloseWhenToldTo() {
        assertTrue(closeChannel(force = true))

        assertEquals(listOf("force close"), calls)
    }

    @Test
    fun passesOnAForceCloseThatFails() {
        assertThrows(IllegalStateException::class.java) {
            EmergencyClose.closeChannel(
                force = true,
                close = {},
                forceClose = { throw IllegalStateException("node not running") },
            )
        }
    }
}
