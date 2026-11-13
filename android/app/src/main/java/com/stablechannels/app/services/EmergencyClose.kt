package com.stablechannels.app.services

import org.lightningdevkit.ldknode.ChannelShutdownState

/**
 * The emergency close (#363): a normal close while the LSP is online, otherwise a force close. LDK
 * refuses a normal close while one is under way, also after a restart or when the LSP started it,
 * so a close under way is forced, as is a normal close that LDK refuses.
 */
object EmergencyClose {
    /** Why the emergency close is off while a move is not final (#456). */
    const val MOVE_PENDING = "Not available until the provider finishes your move."

    /** Whether to force close without trying a normal close first. */
    fun forcesAtOnce(
        closeStarted: Boolean,
        lspOnline: Boolean,
        shutdownState: ChannelShutdownState?,
    ): Boolean =
        closeStarted ||
            !lspOnline ||
            (shutdownState != null && shutdownState != ChannelShutdownState.NOT_SHUTTING_DOWN)

    /**
     * Runs [close], or [forceClose] when [force] or when [close] throws. Returns whether it force
     * closed.
     */
    fun closeChannel(
        force: Boolean,
        close: () -> Unit,
        forceClose: () -> Unit,
        onRefused: (Exception) -> Unit = {},
    ): Boolean {
        if (!force) {
            try {
                close()
                return false
            } catch (e: Exception) {
                onRefused(e)
            }
        }
        forceClose()
        return true
    }
}
