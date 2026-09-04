package com.stablechannels.app

import org.junit.Assert.assertEquals
import org.junit.Test

class OnChainSendBalanceTest {

    @Test
    fun `total balance with active ready channel returns lightning plus onchain`() {
        val total = AppState.calculateTotalBalance(
            lightning = 100_000L,
            onchain = 50_000L,
            hasReady = true,
            isChannelClosing = false,
            isSweeping = false,
            pendingSweep = 0L
        )
        assertEquals(150_000L, total)
    }

    @Test
    fun `total balance during channel closing returns onchain only`() {
        val total = AppState.calculateTotalBalance(
            lightning = 100_000L,
            onchain = 50_000L,
            hasReady = false,
            isChannelClosing = true,
            isSweeping = false,
            pendingSweep = 0L
        )
        assertEquals(50_000L, total)
    }

    @Test
    fun `total balance during sweeping returns lightning`() {
        val total = AppState.calculateTotalBalance(
            lightning = 100_000L,
            onchain = 50_000L,
            hasReady = true,
            isChannelClosing = false,
            isSweeping = true,
            pendingSweep = 0L
        )
        assertEquals(100_000L, total)
    }

    @Test
    fun `total balance without ready channel includes onchain and pending sweep`() {
        val total = AppState.calculateTotalBalance(
            lightning = 80_000L, // Stale claimable from closed channel
            onchain = 45_000L,
            hasReady = false,
            isChannelClosing = false,
            isSweeping = false,
            pendingSweep = 15_000L
        )
        // Stale lightning is ignored, only onchain + pending sweep counted
        assertEquals(60_000L, total)
    }

    @Test
    fun `total balance without ready channel zeroes on send max and never resurrects stale lightning (Issue 260)`() {
        // Bug reproduction: Channel closed, user sent max onchain so onchain drops to 0.
        // LDK continues reporting stale lightning balance.
        // Total balance must be 0, not resurrect the stale lightning balance.
        val total = AppState.calculateTotalBalance(
            lightning = 120_000L, // Stale closed-channel lightning claimables
            onchain = 0L,
            hasReady = false,
            isChannelClosing = false,
            isSweeping = false,
            pendingSweep = 0L
        )
        assertEquals(0L, total)
    }

    @Test
    fun `required confirmations policy returns 1 for splices and 6 for direct onchain sends`() {
        assertEquals(1, AppState.requiredConfirmationsForType("splice_in"))
        assertEquals(1, AppState.requiredConfirmationsForType("splice_out"))
        assertEquals(6, AppState.requiredConfirmationsForType("onchain"))
        assertEquals(6, AppState.requiredConfirmationsForType("channel_close"))
        assertEquals(6, AppState.requiredConfirmationsForType("unknown"))
    }
}
