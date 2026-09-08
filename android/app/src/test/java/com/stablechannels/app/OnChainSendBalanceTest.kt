package com.stablechannels.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
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
    fun `pending outbound send fails closed past TTL without balance drop or confirmation`() {
        val sendTimestamp = 1_700_000_000L
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = sendTimestamp,
            txids = listOf("tx1")
        )

        // Before TTL (at 300 seconds): retains pending send
        val active = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 300L,
            ttlSecs = 600L
        )
        assertEquals(30_000L, active.amountSats)

        // Past TTL (at 601 seconds): fails closed and retains deduction during indexer/node outage
        val pastTtl = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 601L,
            ttlSecs = 600L
        )
        assertEquals(30_000L, pastTtl.amountSats)
    }

    @Test
    fun `incoming deposit with authoritative tx confirmation clears pending without stranding balance`() {
        // Baseline 100k, send 30k. Expected remaining = 70k.
        // But a 50k deposit lands concurrently, raising raw balance to 120k.
        val sendTimestamp = 1_700_000_000L
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = sendTimestamp,
            txids = listOf("tx1")
        )

        val rawWithDeposit = 120_000L

        // Prior to confirmation, balance predicate alone does not clear because 120k > 70k (fails closed)
        val pendingUnconfirmed = AppState.resolvePendingOutboundSend(
            rawOnchain = rawWithDeposit,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 100L,
            ttlSecs = 600L,
            isTxConfirmed = { false }
        )
        assertEquals(30_000L, pendingUnconfirmed.amountSats)

        // Even past TTL, if unconfirmed it fails closed
        val pendingPastTtlUnconfirmed = AppState.resolvePendingOutboundSend(
            rawOnchain = rawWithDeposit,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 650L,
            ttlSecs = 600L,
            isTxConfirmed = { false }
        )
        assertEquals(30_000L, pendingPastTtlUnconfirmed.amountSats)

        // Once authoritative confirmation verifies tx1 is incorporated, pending clears
        val pendingConfirmed = AppState.resolvePendingOutboundSend(
            rawOnchain = rawWithDeposit,
            pending = pending,
            currentTimestampSecs = sendTimestamp + 100L,
            ttlSecs = 600L,
            isTxConfirmed = { it == "tx1" }
        )
        assertEquals(0L, pendingConfirmed.amountSats)
        val (effOnchain, _) = AppState.calculateEffectiveBalances(
            rawOnchain = rawWithDeposit,
            rawSpendable = rawWithDeposit,
            pending = pendingConfirmed
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
        assertEquals("pending_outbound_txids", AppState.Companion.BalanceCacheKey.PENDING_TXIDS)
    }

    @Test
    fun `partial incorporation of multiple sends does not double deduct`() {
        // Baseline 100k, spendable 95k. User performs two sends: 30k, then 20k (total pending = 50k).
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 50_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = 1_000_000L
        )

        // Stage 1: No sends incorporated into raw yet (rawOnchain = 100k)
        val (onchain1, spendable1) = AppState.calculateEffectiveBalances(
            rawOnchain = 100_000L,
            rawSpendable = 95_000L,
            pending = pending
        )
        assertEquals(50_000L, onchain1)
        assertEquals(45_000L, spendable1)

        // Stage 2: Intermediate state - send 1 (30k) has landed in raw balance (rawOnchain = 70k),
        // but send 2 (20k) is still unincorporated.
        // Effective balance must remain 50k, NOT drop to 20k from double-deduction.
        val (onchain2, spendable2) = AppState.calculateEffectiveBalances(
            rawOnchain = 70_000L,
            rawSpendable = 65_000L,
            pending = pending
        )
        assertEquals(50_000L, onchain2)
        assertEquals(45_000L, spendable2)

        // Stage 3: Both sends incorporated into raw balance (rawOnchain = 50k)
        val (onchain3, spendable3) = AppState.calculateEffectiveBalances(
            rawOnchain = 50_000L,
            rawSpendable = 45_000L,
            pending = pending
        )
        assertEquals(50_000L, onchain3)
        assertEquals(45_000L, spendable3)

        val resolved = AppState.resolvePendingOutboundSend(
            rawOnchain = 50_000L,
            pending = pending,
            currentTimestampSecs = 1_000_100L,
            ttlSecs = 600L
        )
        assertEquals(0L, resolved.amountSats)
    }

    @Test
    fun `out of order sync completion cannot clear newer broadcast generation`() {
        // Generation 1 (older broadcast) finishes after Generation 2 was broadcast
        val shouldClearOlder = AppState.shouldClearPendingOnSyncCompletion(
            expectedGeneration = 1L,
            currentGeneration = 2L,
            syncSuccess = true
        )
        assertFalse(shouldClearOlder)

        // Generation 2 (current broadcast) finishes and clears
        val shouldClearCurrent = AppState.shouldClearPendingOnSyncCompletion(
            expectedGeneration = 2L,
            currentGeneration = 2L,
            syncSuccess = true
        )
        assertTrue(shouldClearCurrent)
    }

    @Test
    fun `all sync attempts fail preserves pending deduction`() {
        val shouldClearFailedSync = AppState.shouldClearPendingOnSyncCompletion(
            expectedGeneration = 1L,
            currentGeneration = 1L,
            syncSuccess = false
        )
        assertFalse(shouldClearFailedSync)
    }

    @Test
    fun `authoritative tx failure clears pending and restores spendable balance`() {
        val pending = AppState.Companion.PendingOutboundSend(
            amountSats = 30_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            txids = listOf("tx_failed")
        )

        val resolved = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            isTxFailed = { it == "tx_failed" }
        )
        assertEquals(0L, resolved.amountSats)
        val (effOnchain, effSpendable) = AppState.calculateEffectiveBalances(
            rawOnchain = 100_000L,
            rawSpendable = 95_000L,
            pending = resolved
        )
        assertEquals(100_000L, effOnchain)
        assertEquals(95_000L, effSpendable)
    }

    @Test
    fun `mixed succeeded and failed batch partial release releases only resolved amount`() {
        // Two sends aggregated: tx_fail (30k) and tx_success (20k). Total = 50k.
        val pending = AppState.Companion.PendingOutboundSend(
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = 1_000_000L,
            entries = listOf(
                AppState.Companion.TxEntry("tx_fail", 30_000L),
                AppState.Companion.TxEntry("tx_success", 20_000L)
            )
        )
        assertEquals(50_000L, pending.amountSats)
        assertEquals(listOf("tx_fail", "tx_success"), pending.txids)

        // Case 1: tx_fail fails; tx_success is not yet incorporated.
        // The failed 30k must be released, retaining only 20k for tx_success.
        val partialFail = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            isTxIncorporated = { false },
            isTxFailed = { it == "tx_fail" }
        )
        assertEquals(20_000L, partialFail.amountSats)
        assertEquals(1, partialFail.entries.size)
        assertEquals("tx_success", partialFail.entries.first().txid)

        val (effOnchainFail, effSpendableFail) = AppState.calculateEffectiveBalances(
            rawOnchain = 100_000L,
            rawSpendable = 95_000L,
            pending = partialFail
        )
        assertEquals(80_000L, effOnchainFail)
        assertEquals(75_000L, effSpendableFail)

        // Case 2: tx_success is incorporated; tx_fail is still pending.
        val partialSuccess = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            isTxIncorporated = { it == "tx_success" },
            isTxFailed = { false }
        )
        assertEquals(30_000L, partialSuccess.amountSats)
        assertEquals(1, partialSuccess.entries.size)
        assertEquals("tx_fail", partialSuccess.entries.first().txid)

        // Case 3: Both resolve (one failed, one incorporated). Entire record clears.
        val bothResolved = AppState.resolvePendingOutboundSend(
            rawOnchain = 100_000L,
            pending = pending,
            isTxIncorporated = { it == "tx_success" },
            isTxFailed = { it == "tx_fail" }
        )
        assertEquals(0L, bothResolved.amountSats)
        assertTrue(bothResolved.entries.isEmpty())
    }

    @Test
    fun `relaunch with concurrent deposit clears on incorporation`() {
        // gpt-6-astra P1 scenario:
        // App was killed before sync, restored on relaunch from cache.
        // During relaunch, an incoming deposit raised raw onchain to 120_000 (baseline was 100_000).
        // Since rawOnchain > baseline, rawDrop is 0.
        // The broadcast tx ("tx_mempool") is tracked in LDK with PENDING status (not SUCCEEDED).
        val pending = AppState.Companion.PendingOutboundSend(
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = 1_000_000L,
            entries = listOf(
                AppState.Companion.TxEntry("tx_mempool", 50_000L)
            )
        )

        // Under new policy, isTxIncorporated returns true for PENDING or SUCCEEDED status.
        val resolved = AppState.resolvePendingOutboundSend(
            rawOnchain = 120_000L,
            pending = pending,
            isTxIncorporated = { it == "tx_mempool" },
            isTxFailed = { false }
        )
        assertEquals(0L, resolved.amountSats)

        // Effective balance immediately displays full deposit without stuck deduction.
        val (effOnchain, effSpendable) = AppState.calculateEffectiveBalances(
            rawOnchain = 120_000L,
            rawSpendable = 115_000L,
            pending = resolved
        )
        assertEquals(120_000L, effOnchain)
        assertEquals(115_000L, effSpendable)
    }

    @Test
    fun `per txid entry from legacy deserialization distributes aggregate`() {
        val legacy = AppState.Companion.PendingOutboundSend.fromLegacy(
            amountSats = 50_000L,
            isSendAll = false,
            baselineOnchainSats = 100_000L,
            timestampSecs = 1_000_000L,
            txids = listOf("legacy_tx1", "legacy_tx2")
        )
        assertEquals(50_000L, legacy.amountSats)
        assertEquals(2, legacy.entries.size)
        assertEquals("legacy_tx1", legacy.entries[0].txid)
        assertEquals(25_000L, legacy.entries[0].amountSats)
        assertEquals("legacy_tx2", legacy.entries[1].txid)
        assertEquals(25_000L, legacy.entries[1].amountSats)
    }

    @Test
    fun `recordBroadcast accumulates entries idempotently and maintains sticky sendAll`() {
        var pending = AppState.Companion.PendingOutboundSend()

        // 1. Initial send records entry and sets baseline
        pending = AppState.Companion.recordBroadcast(
            currentPending = pending,
            amountSats = 20_000L,
            isSendAll = false,
            currentOnchain = 100_000L,
            txid = "tx1"
        )
        assertEquals(20_000L, pending.amountSats)
        assertEquals(100_000L, pending.baselineOnchainSats)
        assertEquals(1, pending.entries.size)
        assertEquals("tx1", pending.entries[0].txid)
        assertFalse(pending.isSendAll)

        // 2. Second send appends distinct entry, baseline remains unchanged
        pending = AppState.Companion.recordBroadcast(
            currentPending = pending,
            amountSats = 30_000L,
            isSendAll = false,
            currentOnchain = 80_000L,
            txid = "tx2"
        )
        assertEquals(50_000L, pending.amountSats)
        assertEquals(100_000L, pending.baselineOnchainSats)
        assertEquals(2, pending.entries.size)
        assertEquals("tx2", pending.entries[1].txid)

        // 3. Repeated broadcast with same txid is idempotent
        pending = AppState.Companion.recordBroadcast(
            currentPending = pending,
            amountSats = 30_000L,
            isSendAll = false,
            currentOnchain = 80_000L,
            txid = "tx2"
        )
        assertEquals(50_000L, pending.amountSats)
        assertEquals(2, pending.entries.size)

        // 4. Send all sets isSendAll and records send amount
        var sendAllPending = AppState.Companion.recordBroadcast(
            currentPending = AppState.Companion.PendingOutboundSend(),
            amountSats = 0L,
            isSendAll = true,
            currentOnchain = 100_000L,
            txid = "tx_all"
        )
        assertTrue(sendAllPending.isSendAll)
        assertEquals(100_000L, sendAllPending.amountSats)

        // 5. Subsequent send preserves sticky isSendAll
        sendAllPending = AppState.Companion.recordBroadcast(
            currentPending = sendAllPending,
            amountSats = 10_000L,
            isSendAll = false,
            currentOnchain = 0L,
            txid = "tx_after"
        )
        assertTrue(sendAllPending.isSendAll)

        // 6. Anonymous entry when txid is null
        val anon = AppState.Companion.recordBroadcast(
            currentPending = AppState.Companion.PendingOutboundSend(),
            amountSats = 15_000L,
            isSendAll = false,
            currentOnchain = 50_000L,
            txid = null
        )
        assertEquals(1, anon.entries.size)
        assertEquals("", anon.entries[0].txid)
        assertEquals(15_000L, anon.entries[0].amountSats)
    }

    @Test
    fun `cached pending outbound send roundtrips across formats`() {
        val fakePrefs = FakeSharedPreferences()
        val editor = fakePrefs.edit()

        // 1. New colon format roundtrip
        val original = AppState.Companion.PendingOutboundSend(
            isSendAll = false,
            baselineOnchainSats = 80_000L,
            timestampSecs = 1_700_000_000L,
            entries = listOf(
                AppState.Companion.TxEntry("tx_a", 25_000L),
                AppState.Companion.TxEntry("tx_b", 35_000L)
            )
        )
        AppState.Companion.persistPendingOutboundSend(editor, original)
        editor.apply()

        val loaded = AppState.Companion.loadCachedPendingOutboundSend(fakePrefs)
        assertEquals(original.isSendAll, loaded.isSendAll)
        assertEquals(original.baselineOnchainSats, loaded.baselineOnchainSats)
        assertEquals(original.timestampSecs, loaded.timestampSecs)
        assertEquals(original.amountSats, loaded.amountSats)
        assertEquals(2, loaded.entries.size)
        assertEquals("tx_a", loaded.entries[0].txid)
        assertEquals(25_000L, loaded.entries[0].amountSats)
        assertEquals("tx_b", loaded.entries[1].txid)
        assertEquals(35_000L, loaded.entries[1].amountSats)

        // 2. Anonymous entry roundtrip
        val anonOriginal = AppState.Companion.PendingOutboundSend(
            isSendAll = false,
            baselineOnchainSats = 50_000L,
            timestampSecs = 1_700_000_000L,
            entries = listOf(AppState.Companion.TxEntry("", 50_000L))
        )
        AppState.Companion.persistPendingOutboundSend(editor, anonOriginal)
        editor.apply()
        val loadedAnon = AppState.Companion.loadCachedPendingOutboundSend(fakePrefs)
        assertEquals(1, loadedAnon.entries.size)
        assertEquals("", loadedAnon.entries[0].txid)
        assertEquals(50_000L, loadedAnon.entries[0].amountSats)

        // 3. Legacy comma-delimited fallback without colons
        fakePrefs.clear()
        editor.putLong(AppState.Companion.BalanceCacheKey.PENDING_AMOUNT, 40_000L)
        editor.putBoolean(AppState.Companion.BalanceCacheKey.PENDING_IS_SEND_ALL, false)
        editor.putLong(AppState.Companion.BalanceCacheKey.PENDING_BASELINE, 100_000L)
        editor.putLong(AppState.Companion.BalanceCacheKey.PENDING_TIMESTAMP, 1_700_000_000L)
        editor.putString(AppState.Companion.BalanceCacheKey.PENDING_TXIDS, "leg1,leg2")
        editor.apply()

        val loadedLegacy = AppState.Companion.loadCachedPendingOutboundSend(fakePrefs)
        assertEquals(40_000L, loadedLegacy.amountSats)
        assertEquals(2, loadedLegacy.entries.size)
        assertEquals("leg1", loadedLegacy.entries[0].txid)
        assertEquals(20_000L, loadedLegacy.entries[0].amountSats)
        assertEquals("leg2", loadedLegacy.entries[1].txid)
        assertEquals(20_000L, loadedLegacy.entries[1].amountSats)

        // 4. Zero pending amount when not send-all yields empty entries
        fakePrefs.clear()
        editor.putLong(AppState.Companion.BalanceCacheKey.PENDING_AMOUNT, 0L)
        editor.putBoolean(AppState.Companion.BalanceCacheKey.PENDING_IS_SEND_ALL, false)
        editor.putString(AppState.Companion.BalanceCacheKey.PENDING_TXIDS, "stale_tx")
        editor.apply()
        val loadedZero = AppState.Companion.loadCachedPendingOutboundSend(fakePrefs)
        assertEquals(0L, loadedZero.amountSats)
        assertTrue(loadedZero.entries.isEmpty())
    }

    private class FakeSharedPreferences : android.content.SharedPreferences {
        private val data = mutableMapOf<String, Any>()

        fun clear() {
            data.clear()
        }

        override fun getAll(): Map<String, *> = data
        override fun getString(key: String, defValue: String?): String? = data[key] as? String ?: defValue
        override fun getStringSet(key: String, defValues: Set<String>?): Set<String>? = null
        override fun getInt(key: String, defValue: Int): Int = (data[key] as? Number)?.toInt() ?: defValue
        override fun getLong(key: String, defValue: Long): Long = (data[key] as? Number)?.toLong() ?: defValue
        override fun getFloat(key: String, defValue: Float): Float = (data[key] as? Number)?.toFloat() ?: defValue
        override fun getBoolean(key: String, defValue: Boolean): Boolean = (data[key] as? Boolean) ?: defValue
        override fun contains(key: String): Boolean = data.containsKey(key)
        override fun edit(): android.content.SharedPreferences.Editor = FakeEditor(data)
        override fun registerOnSharedPreferenceChangeListener(listener: android.content.SharedPreferences.OnSharedPreferenceChangeListener?) {}
        override fun unregisterOnSharedPreferenceChangeListener(listener: android.content.SharedPreferences.OnSharedPreferenceChangeListener?) {}

        private class FakeEditor(private val data: MutableMap<String, Any>) : android.content.SharedPreferences.Editor {
            private val temp = mutableMapOf<String, Any>()
            private val removals = mutableSetOf<String>()
            private var clearRequested = false

            override fun putString(key: String, value: String?): android.content.SharedPreferences.Editor {
                if (value != null) temp[key] = value else removals.add(key)
                return this
            }
            override fun putStringSet(key: String, values: Set<String>?): android.content.SharedPreferences.Editor = this
            override fun putInt(key: String, value: Int): android.content.SharedPreferences.Editor {
                temp[key] = value
                return this
            }
            override fun putLong(key: String, value: Long): android.content.SharedPreferences.Editor {
                temp[key] = value
                return this
            }
            override fun putFloat(key: String, value: Float): android.content.SharedPreferences.Editor {
                temp[key] = value
                return this
            }
            override fun putBoolean(key: String, value: Boolean): android.content.SharedPreferences.Editor {
                temp[key] = value
                return this
            }
            override fun remove(key: String): android.content.SharedPreferences.Editor {
                removals.add(key)
                return this
            }
            override fun clear(): android.content.SharedPreferences.Editor {
                clearRequested = true
                return this
            }
            override fun commit(): Boolean {
                apply()
                return true
            }
            override fun apply() {
                if (clearRequested) data.clear()
                removals.forEach { data.remove(it) }
                data.putAll(temp)
            }
        }
    }
}

