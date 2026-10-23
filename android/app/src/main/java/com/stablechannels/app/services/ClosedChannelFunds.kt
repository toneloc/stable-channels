package com.stablechannels.app.services

import org.lightningdevkit.ldknode.BalanceSource
import org.lightningdevkit.ldknode.LightningBalance
import org.lightningdevkit.ldknode.PendingSweepBalance

/**
 * Funds from a closed channel that are not in the onchain wallet yet (#358). After a force close
 * they are timelocked on chain, then LDK sweeps them to the wallet. A closed channel is reported as
 * ClaimableOnChannelClose until its close tx confirms, so that is counted only for channels no
 * longer open. Confirmed coop close funds are left out: they are already in the wallet. A sweep
 * stays counted until it confirms, as the wallet may not show it before then.
 */
data class ClosedChannelFunds(val amountSats: Long, val blocksLeft: Int?) {
    companion object {
        private const val MINUTES_PER_BLOCK = 10

        /** "1 block (about 10 minutes)", "288 blocks (about 2 days)", rounding up. */
        fun blocksText(blocks: Int): String {
            val minutes = blocks * MINUTES_PER_BLOCK
            val about =
                when {
                    minutes < 60 -> "$minutes minutes"
                    minutes < 24 * 60 -> count((minutes + 59) / 60, "hour")
                    else -> count((minutes + 24 * 60 - 1) / (24 * 60), "day")
                }
            return "${count(blocks, "block")} (about $about)"
        }

        private fun count(n: Int, unit: String) = if (n == 1) "1 $unit" else "$n ${unit}s"

        fun from(
            lightningBalances: List<LightningBalance>,
            pendingSweeps: List<PendingSweepBalance>,
            openChannelIds: Set<String>,
            currentHeight: Int,
        ): ClosedChannelFunds? {
            var amountSats = 0L
            var unlockHeight: Int? = null
            for (balance in lightningBalances) {
                when (balance) {
                    is LightningBalance.ClaimableOnChannelClose -> {
                        if (balance.channelId in openChannelIds) continue
                        amountSats += balance.amountSatoshis.toLong()
                    }
                    is LightningBalance.ClaimableAwaitingConfirmations -> {
                        if (balance.source == BalanceSource.COOP_CLOSE) continue
                        amountSats += balance.amountSatoshis.toLong()
                        val height = balance.confirmationHeight.toInt()
                        unlockHeight = maxOf(unlockHeight ?: height, height)
                    }
                    is LightningBalance.ContentiousClaimable ->
                        amountSats += balance.amountSatoshis.toLong()
                    is LightningBalance.MaybeTimeoutClaimableHtlc ->
                        amountSats += balance.amountSatoshis.toLong()
                    is LightningBalance.MaybePreimageClaimableHtlc ->
                        amountSats += balance.amountSatoshis.toLong()
                    is LightningBalance.CounterpartyRevokedOutputClaimable ->
                        amountSats += balance.amountSatoshis.toLong()
                }
            }
            for (sweep in pendingSweeps) {
                when (sweep) {
                    is PendingSweepBalance.PendingBroadcast ->
                        amountSats += sweep.amountSatoshis.toLong()
                    is PendingSweepBalance.BroadcastAwaitingConfirmation ->
                        amountSats += sweep.amountSatoshis.toLong()
                    is PendingSweepBalance.AwaitingThresholdConfirmations -> {}
                }
            }
            if (amountSats <= 0L) return null
            return ClosedChannelFunds(
                amountSats = amountSats,
                blocksLeft = unlockHeight?.let { maxOf(it - currentHeight, 0) },
            )
        }
    }
}
