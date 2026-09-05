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
            pendingSweep = 0L,
            hasAnyChannel = false
        )
        assertEquals(0L, total)
    }

    @Test
    fun `total balance with pending channel when hasReady is false preserves lightning balance`() {
        // A channel exists (hasAnyChannel = true) but is not ready yet (funding pending)
        val total = AppState.calculateTotalBalance(
            lightning = 80_000L,
            onchain = 20_000L,
            hasReady = false,
            isChannelClosing = false,
            isSweeping = false,
            pendingSweep = 0L,
            hasAnyChannel = true
        )
        assertEquals(100_000L, total)
    }

    @Test
    fun `total balance during channel opening returns lightning if non-zero else onchain`() {
        val totalWithLightning = AppState.calculateTotalBalance(
            lightning = 50_000L,
            onchain = 0L,
            hasReady = false,
            isChannelClosing = false,
            isSweeping = false,
            isOpeningChannel = true
        )
        assertEquals(50_000L, totalWithLightning)

        val totalWithOnchain = AppState.calculateTotalBalance(
            lightning = 0L,
            onchain = 70_000L,
            hasReady = false,
            isChannelClosing = false,
            isSweeping = false,
            isOpeningChannel = true
        )
        assertEquals(70_000L, totalWithOnchain)
    }

    @Test
    fun `total balance using ChannelState parameter object adheres to Open Closed Principle`() {
        val state = AppState.Companion.ChannelState(
            hasReady = false,
            hasAnyChannel = false,
            isChannelClosing = false,
            isOpeningChannel = false,
            isSweeping = false
        )
        val total = AppState.calculateTotalBalance(
            lightning = 50_000L,
            onchain = 20_000L,
            pendingSweep = 5_000L,
            channelState = state
        )
        // Without ready or existing channel, stale lightning is excluded
        assertEquals(25_000L, total)
    }

    @Test
    fun `required confirmations policy returns 1 for splices and 6 for direct onchain sends`() {
        assertEquals(1, AppState.requiredConfirmationsForType("splice_in"))
        assertEquals(1, AppState.requiredConfirmationsForType("splice_out"))
        assertEquals(6, AppState.requiredConfirmationsForType("onchain"))
        assertEquals(6, AppState.requiredConfirmationsForType("channel_close"))
        assertEquals(6, AppState.requiredConfirmationsForType("unknown"))
    }

    @Test
    fun `calculate effective balances deducts pending outbound send`() {
        val pendingPartial = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L
        )
        val (onchainPartial, spendablePartial) = AppState.calculateEffectiveBalances(
            rawOnchain = 100_000L,
            rawSpendable = 95_000L,
            pending = pendingPartial
        )
        assertEquals(70_000L, onchainPartial)
        assertEquals(65_000L, spendablePartial)

        val pendingAll = AppState.Companion.PendingOutboundSend(
            amountSats = 100_000L,
            isSendAll = true,
            baselineOnchainSats = 100_000L
        )
        val (onchainAll, spendableAll) = AppState.calculateEffectiveBalances(
            rawOnchain = 100_000L,
            rawSpendable = 95_000L,
            pending = pendingAll
        )
        assertEquals(0L, onchainAll)
        assertEquals(0L, spendableAll)

        val pendingNone = AppState.Companion.PendingOutboundSend()
        val (onchainNone, spendableNone) = AppState.calculateEffectiveBalances(
            rawOnchain = 100_000L,
            rawSpendable = 95_000L,
            pending = pendingNone
        )
        assertEquals(100_000L, onchainNone)
        assertEquals(95_000L, spendableNone)
    }

    @Test
    fun `resolve pending outbound send clears only after spend is incorporated`() {
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L
        )

        // Before wallet sync incorporates the spend: raw balance is still 100k
        val stillPending = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending
        )
        assertEquals(30_000L, stillPending.amountSats)
        assertEquals(false, stillPending.isSendAll)

        // After wallet sync incorporates the spend: raw balance dropped to 70k or less
        val incorporated = AppState.resolvePendingOutboundSend(
            rawOnchain = 69_850L,
            pending = pending
        )
        assertEquals(0L, incorporated.amountSats)

        // Send All incorporation
        val pendingAll = AppState.Companion.PendingOutboundSend(
            amountSats = 100_000L,
            isSendAll = true,
            baselineOnchainSats = 100_000L
        )
        val allStillPending = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pendingAll
        )
        assertEquals(true, allStillPending.isSendAll)

        val allIncorporated = AppState.resolvePendingOutboundSend(
            rawOnchain = 0L,
            pending = pendingAll
        )
        assertEquals(false, allIncorporated.isSendAll)
        assertEquals(0L, allIncorporated.amountSats)
    }

    @Test
    fun `interleaving balance refresh before wallet sync completes preserves deduction and prevents ghost balance resurrection`() {
        // Simulates:
        // 1. Broadcast send of 30,000 sats when raw balance is 100,000 sats.
        // 2. Intervening refresh receives old 100,000 balance from LDK before sync incorporates the spend.
        // 3. Invariant: effective balance MUST remain 70,000 and never resurrect the pre-send 100,000.
        val rawPreSend = 100_000L
        val rawSpendablePreSend = 95_000L
        val sendAmount = 30_000L

        var pending = AppState.Companion.PendingOutboundSend(
            amountSats = sendAmount,
            isSendAll = false,
            baselineOnchainSats = rawPreSend
        )

        // Intervening refresh: LDK still reports old 100k balance
        val rawDuringSync = rawPreSend
        val rawSpendableDuringSync = rawSpendablePreSend

        pending = AppState.resolvePendingOutboundSend(rawOnchain = rawDuringSync, pending = pending)
        assertEquals(30_000L, pending.amountSats)

        val (effOnchainDuringSync, effSpendableDuringSync) = AppState.calculateEffectiveBalances(
            rawOnchain = rawDuringSync,
            rawSpendable = rawSpendableDuringSync,
            pending = pending
        )
        assertEquals(70_000L, effOnchainDuringSync)
        assertEquals(65_000L, effSpendableDuringSync)

        // Once sync completes and LDK reports the incorporated balance:
        val rawPostSync = 69_850L
        val rawSpendablePostSync = 64_850L

        pending = AppState.resolvePendingOutboundSend(rawOnchain = rawPostSync, pending = pending)
        assertEquals(0L, pending.amountSats)

        val (effOnchainPostSync, effSpendablePostSync) = AppState.calculateEffectiveBalances(
            rawOnchain = rawPostSync,
            rawSpendable = rawSpendablePostSync,
            pending = pending
        )
        assertEquals(69_850L, effOnchainPostSync)
        assertEquals(64_850L, effSpendablePostSync)
    }
}
