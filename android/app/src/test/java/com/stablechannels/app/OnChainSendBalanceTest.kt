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

    @Test
    fun `overlapping consecutive sends does not double deduct`() {
        val initialRaw = 100_000L
        val initialSpendable = 95_000L

        // Send 1: 30,000 sats
        val send1 = 30_000L
        val pending1 = AppState.Companion.PendingOutboundSend(
            amountSats = send1,
            isSendAll = false,
            baselineOnchainSats = initialRaw
        )
        val (eff1Onchain, eff1Spendable) = AppState.calculateEffectiveBalances(
            rawOnchain = initialRaw,
            rawSpendable = initialSpendable,
            pending = pending1
        )
        assertEquals(70_000L, eff1Onchain)
        assertEquals(65_000L, eff1Spendable)

        // Send 2: 20,000 sats issued while Send 1 is still pending
        val send2 = 20_000L
        val cumulativePending = AppState.Companion.PendingOutboundSend(
            amountSats = pending1.amountSats + send2,
            isSendAll = false,
            baselineOnchainSats = pending1.baselineOnchainSats
        )
        assertEquals(50_000L, cumulativePending.amountSats)
        assertEquals(initialRaw, cumulativePending.baselineOnchainSats)

        // Incremental deduction from published balance yields correct 50k (not 20k)
        val incrementalOnchain = (eff1Onchain - send2).coerceAtLeast(0L)
        val incrementalSpendable = (eff1Spendable - send2).coerceAtLeast(0L)
        assertEquals(50_000L, incrementalOnchain)
        assertEquals(45_000L, incrementalSpendable)

        // And effective balance calculation against the raw baseline is consistent
        val (eff2Onchain, eff2Spendable) = AppState.calculateEffectiveBalances(
            rawOnchain = initialRaw,
            rawSpendable = initialSpendable,
            pending = cumulativePending
        )
        assertEquals(50_000L, eff2Onchain)
        assertEquals(45_000L, eff2Spendable)

        // After wallet sync incorporates both sends (raw on-chain drops to 49,850 sats):
        val rawAfterBothSync = 49_850L
        val resolved = AppState.resolvePendingOutboundSend(
            rawOnchain = rawAfterBothSync,
            pending = cumulativePending
        )
        assertEquals(0L, resolved.amountSats)
        val (finalOnchain, _) = AppState.calculateEffectiveBalances(
            rawOnchain = rawAfterBothSync,
            rawSpendable = 44_850L,
            pending = resolved
        )
        assertEquals(49_850L, finalOnchain)
    }

    @Test
    fun `pending outbound send expires after TTL backstop`() {
        val sendTimestamp = 1_700_000_000L
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = sendTimestamp
        )

        // Before TTL (e.g. at 300 seconds): still pending if raw balance hasn't dropped
        val active = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 300L,
            ttlSecs = 600L
        )
        assertEquals(30_000L, active.amountSats)

        // After TTL (at 601 seconds): expired to prevent permanent balance suppression
        val expired = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 601L,
            ttlSecs = 600L
        )
        assertEquals(0L, expired.amountSats)
    }

    @Test
    fun `incoming deposit during pending window clears via TTL without stranding balance`() {
        // Baseline 100k, send 30k. Expected remaining = 70k.
        // But a 50k deposit lands concurrently, raising raw balance to 120k.
        val sendTimestamp = 1_700_000_000L
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = sendTimestamp
        )

        val rawWithDeposit = 120_000L

        // Prior to TTL, the balance predicate alone cannot clear because 120k > 70k
        val pendingBeforeTtl = AppState.resolvePendingOutboundSend(
            rawOnchain = rawWithDeposit,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 100L,
            ttlSecs = 600L
        )
        assertEquals(30_000L, pendingBeforeTtl.amountSats)

        // Once TTL expires, pending clears and user sees the true updated deposit balance
        val pendingAfterTtl = AppState.resolvePendingOutboundSend(
            rawOnchain = rawWithDeposit,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 650L,
            ttlSecs = 600L
        )
        assertEquals(0L, pendingAfterTtl.amountSats)
        val (effOnchain, _) = AppState.calculateEffectiveBalances(
            rawOnchain = rawWithDeposit,
            rawSpendable = rawWithDeposit,
            pending = pendingAfterTtl
        )
        assertEquals(120_000L, effOnchain)
    }

    @Test
    fun `partial then send all sequence preserves baseline and clears on zero`() {
        val initialRaw = 100_000L
        val initialSpendable = 95_000L

        // Send 1: partial 20k
        val pending1 = AppState.Companion.PendingOutboundSend(
            amountSats = 20_000L,
            isSendAll = false,
            baselineOnchainSats = initialRaw
        )
        val (eff1Onchain, eff1Spendable) = AppState.calculateEffectiveBalances(
            rawOnchain = initialRaw,
            rawSpendable = initialSpendable,
            pending = pending1
        )
        assertEquals(80_000L, eff1Onchain)
        assertEquals(75_000L, eff1Spendable)

        // Send 2: send all (remaining 80k)
        val pendingSendAll = AppState.Companion.PendingOutboundSend(
            amountSats = pending1.amountSats + eff1Onchain,
            isSendAll = true,
            baselineOnchainSats = pending1.baselineOnchainSats
        )
        assertEquals(100_000L, pendingSendAll.amountSats)
        assertEquals(initialRaw, pendingSendAll.baselineOnchainSats)

        val (effAllOnchain, effAllSpendable) = AppState.calculateEffectiveBalances(
            rawOnchain = initialRaw,
            rawSpendable = initialSpendable,
            pending = pendingSendAll
        )
        assertEquals(0L, effAllOnchain)
        assertEquals(0L, effAllSpendable)

        // Wallet refresh incorporating only partial send 1 (raw drops to 80k) MUST NOT clear send all
        val refreshAfterPartialOnly = AppState.resolvePendingOutboundSend(
            rawOnchain = 80_000L,
            pending = pendingSendAll
        )
        assertEquals(true, refreshAfterPartialOnly.isSendAll)

        // Once send-all is incorporated (raw drops to 0):
        val fullyResolved = AppState.resolvePendingOutboundSend(
            rawOnchain = 0L,
            pending = pendingSendAll
        )
        assertEquals(false, fullyResolved.isSendAll)
        assertEquals(0L, fullyResolved.amountSats)
    }

    @Test
    fun `centralized balance cache keys match expected preferences schema`() {
        assertEquals("balance_cache", AppState.Companion.BalanceCacheKey.PREFS_NAME)
        assertEquals("cached_lightning_sats", AppState.Companion.BalanceCacheKey.LIGHTNING)
        assertEquals("cached_onchain_sats", AppState.Companion.BalanceCacheKey.ONCHAIN)
        assertEquals("cached_spendable_sats", AppState.Companion.BalanceCacheKey.SPENDABLE)
        assertEquals("pending_outbound_onchain_sats", AppState.Companion.BalanceCacheKey.PENDING_AMOUNT)
        assertEquals("pending_outbound_is_send_all", AppState.Companion.BalanceCacheKey.PENDING_IS_SEND_ALL)
        assertEquals("pending_outbound_baseline_sats", AppState.Companion.BalanceCacheKey.PENDING_BASELINE)
        assertEquals("pending_outbound_timestamp_secs", AppState.Companion.BalanceCacheKey.PENDING_TIMESTAMP)
    }
}
