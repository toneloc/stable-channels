package com.stablechannels.app.services

import org.lightningdevkit.ldknode.BalanceSource
import org.lightningdevkit.ldknode.ClosureReason
import org.lightningdevkit.ldknode.LightningBalance
import org.lightningdevkit.ldknode.PendingSweepBalance

/**
 * Funds from a closed channel that are not in the onchain wallet yet (#358). After a force close
 * they are timelocked on chain, then LDK sweeps them to the wallet. Open channels are left out. A
 * coop close pays straight to the wallet, so it is left out whether or not it has confirmed. A
 * sweep is counted until LDK broadcasts it: the wallet counts it from its next sync.
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

        /**
         * Sweeps LDK has not broadcast yet. A broadcast sweep is left out, as the wallet counts it
         * once it sees the tx. LDK re-signs an unconfirmed sweep each block to a new wallet
         * address, so its latest txid is often not the one the mempool and the wallet have.
         */
        fun unbroadcastSweepSats(pendingSweeps: List<PendingSweepBalance>): Long =
            pendingSweeps.filterIsInstance<PendingSweepBalance.PendingBroadcast>().sumOf {
                it.amountSatoshis.toLong()
            }

        fun isCooperative(reason: ClosureReason?): Boolean =
            reason is ClosureReason.LegacyCooperativeClosure ||
                reason is ClosureReason.CounterpartyInitiatedCooperativeClosure ||
                reason is ClosureReason.LocallyInitiatedCooperativeClosure

        fun from(
            lightningBalances: List<LightningBalance>,
            pendingSweeps: List<PendingSweepBalance>,
            openChannelIds: Set<String>,
            coopClosedChannelIds: Set<String>,
            currentHeight: Int,
        ): ClosedChannelFunds? {
            var amountSats = 0L
            var unlockHeight: Int? = null
            for (balance in lightningBalances) {
                when (balance) {
                    // The wallet shows a coop close as soon as it sees the close tx.
                    is LightningBalance.ClaimableOnChannelClose -> {
                        val id = balance.channelId
                        if (id in openChannelIds || id in coopClosedChannelIds) continue
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
                    // Counted as LDK's claimable_amount_satoshis counts them: an HTLC of our own
                    // payment, or one waiting for a preimage, may never come back to us. An open
                    // channel lists its pending HTLCs too.
                    is LightningBalance.MaybeTimeoutClaimableHtlc -> {
                        if (balance.outboundPayment || balance.channelId in openChannelIds) continue
                        amountSats += balance.amountSatoshis.toLong()
                    }
                    is LightningBalance.MaybePreimageClaimableHtlc -> {}
                    is LightningBalance.CounterpartyRevokedOutputClaimable ->
                        amountSats += balance.amountSatoshis.toLong()
                }
            }
            amountSats += unbroadcastSweepSats(pendingSweeps)
            if (amountSats <= 0L) return null
            return ClosedChannelFunds(
                amountSats = amountSats,
                blocksLeft = unlockHeight?.let { maxOf(it - currentHeight, 0) },
            )
        }
    }
}
