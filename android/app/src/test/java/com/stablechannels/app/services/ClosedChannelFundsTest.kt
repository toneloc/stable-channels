package com.stablechannels.app.services

import com.stablechannels.app.AppState
import com.stablechannels.app.ui.home.ClosedChannelFundsMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.lightningdevkit.ldknode.BalanceSource
import org.lightningdevkit.ldknode.ClosureReason
import org.lightningdevkit.ldknode.LightningBalance
import org.lightningdevkit.ldknode.PendingSweepBalance

class ClosedChannelFundsTest {
    private val peer = "02" + "a".repeat(64)

    private fun onClose(channelId: String, sats: Long) =
        LightningBalance.ClaimableOnChannelClose(
            channelId,
            peer,
            sats.toULong(),
            0uL,
            0uL,
            0uL,
            0uL,
            0uL,
        )

    private fun awaiting(sats: Long, height: Int, source: BalanceSource) =
        LightningBalance.ClaimableAwaitingConfirmations(
            "closed",
            peer,
            sats.toULong(),
            height.toUInt(),
            source,
        )

    private fun from(
        lightning: List<LightningBalance> = emptyList(),
        sweeps: List<PendingSweepBalance> = emptyList(),
        open: Set<String> = emptySet(),
        coopClosed: Set<String> = emptySet(),
        height: Int = 900_000,
    ) = ClosedChannelFunds.from(lightning, sweeps, open, coopClosed, height)

    @Test
    fun openChannelIsNotCounted() {
        assertNull(
            from(
                listOf(onClose("open", 50_000)),
                emptyList(),
                setOf("open"),
                height = 900_000,
            )
        )
    }

    @Test
    fun closedChannelBeforeCloseTxConfirmsIsCounted() {
        val funds =
            from(
                listOf(onClose("closed", 50_000)),
                emptyList(),
                emptySet(),
                height = 900_000,
            )
        assertEquals(ClosedChannelFunds(50_000, null), funds)
    }

    @Test
    fun holderForceCloseReportsBlocksLeft() {
        val funds =
            from(
                listOf(awaiting(40_000, 900_144, BalanceSource.HOLDER_FORCE_CLOSED)),
                emptyList(),
                emptySet(),
                height = 900_000,
            )
        assertEquals(ClosedChannelFunds(40_000, 144), funds)
    }

    @Test
    fun counterpartyForceCloseIsCounted() {
        val funds =
            from(
                listOf(awaiting(40_000, 900_001, BalanceSource.COUNTERPARTY_FORCE_CLOSED)),
                emptyList(),
                emptySet(),
                height = 900_000,
            )
        assertEquals(ClosedChannelFunds(40_000, 1), funds)
    }

    @Test
    fun confirmedCoopCloseIsNotCounted() {
        assertNull(
            from(
                listOf(awaiting(40_000, 900_005, BalanceSource.COOP_CLOSE)),
                emptyList(),
                emptySet(),
                height = 900_000,
            )
        )
    }

    @Test
    fun sweepIsCountedUntilItIsBroadcast() {
        val sweeps =
            listOf(
                PendingSweepBalance.PendingBroadcast("closed", 39_000uL),
                PendingSweepBalance.BroadcastAwaitingConfirmation(
                    "closed",
                    900_010u,
                    "tx",
                    1_000uL,
                ),
            )
        assertEquals(
            ClosedChannelFunds(39_000, null, sweepSats = 39_000),
            from(sweeps = sweeps, height = 900_020),
        )
    }

    @Test
    fun blocksLeftNeverGoesNegative() {
        val funds =
            from(
                listOf(awaiting(40_000, 899_990, BalanceSource.HOLDER_FORCE_CLOSED)),
                emptyList(),
                emptySet(),
                height = 900_000,
            )
        assertEquals(0, funds?.blocksLeft)
    }

    @Test
    fun messageShowsAmountAndTimeLeft() {
        assertEquals(
            "\$40.00 is safe on the Bitcoin blockchain. It unlocks in 288 blocks (about 2 days), then moves to your onchain wallet automatically. Keep the app installed until it arrives.",
            ClosedChannelFundsMessage.text(ClosedChannelFunds(40_000, 288), 100_000.0),
        )
        assertEquals(
            "\$40.00 is safe on the Bitcoin blockchain. It moves to your onchain wallet automatically, usually within a few hours. Keep the app installed until it arrives.",
            ClosedChannelFundsMessage.text(ClosedChannelFunds(40_000, null), 100_000.0),
        )
    }

    @Test
    fun blocksTextUsesTheClosestUnit() {
        assertEquals("1 block (about 10 minutes)", ClosedChannelFunds.blocksText(1))
        assertEquals("6 blocks (about 1 hour)", ClosedChannelFunds.blocksText(6))
        assertEquals("7 blocks (about 2 hours)", ClosedChannelFunds.blocksText(7))
        assertEquals("144 blocks (about 1 day)", ClosedChannelFunds.blocksText(144))
        assertEquals("288 blocks (about 2 days)", ClosedChannelFunds.blocksText(288))
    }

    /** Case A: the LSP force-closes and LDK, which still has the channel data, sweeps the funds. */
    @Test
    fun lspForceCloseIsCountedUntilTheSweepIsBroadcast() {
        assertNull(from(listOf(onClose("closed", 50_000)), open = setOf("closed")))
        // Closed, the close tx not confirmed yet.
        assertEquals(ClosedChannelFunds(50_000, null), from(listOf(onClose("closed", 50_000))))
        // Close confirmed: our output unlocks one block later.
        assertEquals(
            ClosedChannelFunds(50_000, 1),
            from(listOf(awaiting(50_000, 900_001, BalanceSource.COUNTERPARTY_FORCE_CLOSED))),
        )
        // LDK sweeps it to the wallet.
        assertEquals(
            ClosedChannelFunds(49_800, null, sweepSats = 49_800),
            from(sweeps = listOf(PendingSweepBalance.PendingBroadcast("closed", 49_800uL))),
        )
        // Sweep broadcast: the wallet counts it from its next sync.
        assertNull(
            from(
                sweeps =
                    listOf(
                        PendingSweepBalance.BroadcastAwaitingConfirmation(
                            "closed",
                            900_002u,
                            "sweep",
                            49_800uL,
                        )
                    )
            )
        )
        assertNull(
            from(
                sweeps =
                    listOf(
                        PendingSweepBalance.AwaitingThresholdConfirmations(
                            "closed",
                            "sweep",
                            "block",
                            900_003u,
                            49_800uL,
                        )
                    )
            )
        )
    }

    /** Case A, closed from the app: our output is locked for the delay the LSP set (144 blocks). */
    @Test
    fun ownForceCloseShowsTheTimeLeft() {
        val funds = from(listOf(awaiting(50_000, 900_144, BalanceSource.HOLDER_FORCE_CLOSED)))
        assertEquals(ClosedChannelFunds(50_000, 144), funds)
        assertEquals(
            "\$50.00 is safe on the Bitcoin blockchain. It unlocks in 144 blocks (about 1 day), then moves to your onchain wallet automatically. Keep the app installed until it arrives.",
            ClosedChannelFundsMessage.text(funds!!, 100_000.0),
        )
    }

    @Test
    fun pendingHtlcsOfAClosedChannelAreCountedAsLdkCountsThem() {
        val funds =
            from(
                listOf(
                    LightningBalance.ContentiousClaimable(
                        "closed",
                        peer,
                        1_000uL,
                        900_010u,
                        "h",
                        "p",
                    ),
                    // Our own payment: the recipient may claim it.
                    LightningBalance.MaybeTimeoutClaimableHtlc(
                        "closed",
                        peer,
                        2_000uL,
                        900_010u,
                        "h",
                        true,
                    ),
                    LightningBalance.MaybeTimeoutClaimableHtlc(
                        "closed",
                        peer,
                        500uL,
                        900_010u,
                        "h",
                        false,
                    ),
                    LightningBalance.MaybePreimageClaimableHtlc(
                        "closed",
                        peer,
                        3_000uL,
                        900_010u,
                        "h",
                    ),
                    LightningBalance.CounterpartyRevokedOutputClaimable("closed", peer, 4_000uL),
                )
            )
        assertEquals(ClosedChannelFunds(5_500, null), funds)
    }

    @Test
    fun pendingHtlcsOfAnOpenChannelAreNotCounted() {
        val funds =
            from(
                listOf(
                    onClose("open", 50_000),
                    LightningBalance.MaybeTimeoutClaimableHtlc(
                        "open",
                        peer,
                        2_000uL,
                        900_010u,
                        "h",
                        false,
                    ),
                    LightningBalance.MaybePreimageClaimableHtlc(
                        "open",
                        peer,
                        3_000uL,
                        900_010u,
                        "h",
                    ),
                ),
                open = setOf("open"),
            )
        assertNull(funds)
    }

    @Test
    fun aCooperativeCloseIsNotCountedBeforeItConfirms() {
        assertNull(from(listOf(onClose("closed", 50_000)), coopClosed = setOf("closed")))
        assertEquals(
            ClosedChannelFunds(40_000, null),
            from(
                listOf(onClose("coop", 50_000), onClose("forced", 40_000)),
                coopClosed = setOf("coop"),
            ),
        )
    }

    @Test
    fun onlyCooperativeClosuresAreCooperative() {
        assertTrue(
            ClosedChannelFunds.isCooperative(ClosureReason.LocallyInitiatedCooperativeClosure)
        )
        assertTrue(
            ClosedChannelFunds.isCooperative(ClosureReason.CounterpartyInitiatedCooperativeClosure)
        )
        assertTrue(ClosedChannelFunds.isCooperative(ClosureReason.LegacyCooperativeClosure))
        assertFalse(ClosedChannelFunds.isCooperative(ClosureReason.HolderForceClosed(true, "m")))
        assertFalse(ClosedChannelFunds.isCooperative(ClosureReason.CounterpartyForceClosed("m")))
        assertFalse(ClosedChannelFunds.isCooperative(null))
    }

    /** The total adds these to the wallet balance, so a broadcast sweep must not be among them. */
    @Test
    fun onlySweepsNotYetBroadcastAreAdded() {
        val sweeps =
            listOf(
                PendingSweepBalance.PendingBroadcast("a", 39_000uL),
                PendingSweepBalance.BroadcastAwaitingConfirmation("b", 900_002u, "sweep", 6_188uL),
                PendingSweepBalance.AwaitingThresholdConfirmations(
                    "c",
                    "sweep",
                    "block",
                    900_003u,
                    1_000uL,
                ),
            )
        assertEquals(39_000L, ClosedChannelFunds.unbroadcastSweepSats(sweeps))
    }

    @Test
    fun aCooperativeCloseNeverShowsOnceConfirmed() {
        assertNull(from(listOf(awaiting(50_000, 900_001, BalanceSource.COOP_CLOSE))))
    }

    @Test
    fun theLastOfSeveralUnlocksIsShown() {
        val funds =
            from(
                listOf(
                    awaiting(30_000, 900_010, BalanceSource.HOLDER_FORCE_CLOSED),
                    awaiting(20_000, 900_144, BalanceSource.HOLDER_FORCE_CLOSED),
                )
            )
        assertEquals(ClosedChannelFunds(50_000, 144), funds)
    }

    /**
     * A closed channel has 50,000 sats in LDK's lightning balance and a 30,000 sat sweep LDK has
     * not broadcast. Whatever the channel state, the Home total and the card count each sat once.
     */
    @Test
    fun theCardShowsOnlyWhatTheTotalLeavesOut() {
        val funds = ClosedChannelFunds(80_000, 10, sweepSats = 30_000)
        fun home(state: AppState.Companion.ChannelState, openChannelSats: Long = 0L) =
            (openChannelSats + 50_000).let { lightning ->
                AppState.calculateTotalBalance(
                    lightning = lightning,
                    onchain = 20_000,
                    pendingSweep = 30_000,
                    channelState = state,
                ) to AppState.closedChannelFundsLeftOut(funds, lightning, state)?.amountSats
            }

        // No channel: the total counts the sweep, the card the rest.
        assertEquals(50_000L to 50_000L, home(AppState.Companion.ChannelState(hasReady = false)))
        // A channel open: the total counts the lightning balance, the card the sweep.
        assertEquals(
            170_000L to 30_000L,
            home(AppState.Companion.ChannelState(hasReady = true, hasAnyChannel = true), 100_000),
        )
        // Closing: the total is the onchain balance alone.
        assertEquals(
            20_000L to 80_000L,
            home(AppState.Companion.ChannelState(hasReady = false, isChannelClosing = true)),
        )
        // Opening, and moving onchain funds into the channel: the lightning balance alone.
        assertEquals(
            50_000L to 30_000L,
            home(AppState.Companion.ChannelState(hasReady = false, isOpeningChannel = true)),
        )
        assertEquals(
            150_000L to 30_000L,
            home(
                AppState.Companion.ChannelState(
                    hasReady = true,
                    hasAnyChannel = true,
                    isSweeping = true,
                ),
                100_000,
            ),
        )
    }
}
