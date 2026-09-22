package com.stablechannels.app.services

import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.util.Constants
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.roundToLong
import org.lightningdevkit.ldknode.ChannelDetails

object StabilityService {

    /** Below this the position counts as zero-target; a zero target with no backing is closed. */
    const val MINIMUM_STABLE_USD = 0.01

    enum class StabilityAction(val value: String) {
        STABLE("STABLE"),
        HIGH_RISK_NO_ACTION("HIGH_RISK_NO_ACTION"),
        CHECK_ONLY("CHECK_ONLY"),
        PAY("PAY"),
    }

    data class StabilityCheckResult(
        val action: StabilityAction,
        val percentFromPar: Double,
        val stableUSDValue: Double,
        val targetUSD: Double,
        val dollarsFromPar: Double,
    )

    fun reconcileOutgoing(sc: StableChannel, price: Double): Pair<StableChannel, Double?> {
        val updated = sc.copy()
        if (updated.backingSats == 0L || price == 0.0) {
            return Pair(updated, null)
        }
        if (updated.backingSats <= updated.stableReceiverBTC.sats) {
            return Pair(updated, null)
        }
        val overflowSats = updated.backingSats - updated.stableReceiverBTC.sats
        val usdToDeduct = (overflowSats.toDouble() / Constants.SATS_IN_BTC) * price
        val newExpected = max(updated.expectedUSD.amount - usdToDeduct, 0.0)
        updated.expectedUSD = USD(newExpected)
        // Preserve sats, don't re-peg. The overflow is exactly what left the channel, so the
        // sats that remain are the backing. Re-pegging to newExpected/price left backing ABOVE
        // the live balance whenever the position was below par — so a retry deducted again
        // ($100 -> $92 -> $82), and the leftover phantom backing masked a real below-par claim
        // from the stability check. This mirrors the LSP (backing_after_user_to_lsp_stability)
        // and makes the function idempotent: re-running sees backing <= receiver and returns.
        // At the zero boundary the residue stays backing: a $0 target with sats still backing it
        // is an unsettled LSP surplus (#322), which checkStabilityAction now settles as a normal
        // above-par PAY. Zeroing it here would release the LSP's sats to the user as native BTC.
        updated.backingSats = updated.stableReceiverBTC.sats
        recomputeNative(updated)
        return Pair(updated, usdToDeduct)
    }

    fun reconcileIncoming(sc: StableChannel): StableChannel {
        val updated = sc.copy()
        recomputeNative(updated)
        return updated
    }

    fun applyTrade(sc: StableChannel, newExpectedUSD: Double, price: Double): StableChannel {
        val updated = sc.copy()
        updated.expectedUSD = USD(newExpectedUSD)
        if (price > 0) {
            updated.backingSats = ((newExpectedUSD / price) * Constants.SATS_IN_BTC).roundToLong()
        }
        recomputeNative(updated)
        return updated
    }

    /**
     * Whether a spend of [amountMsat] would consume backing already owed to the LSP (#322).
     * Only the excess over the native (non-backing) balance AND the stable target itself touches
     * the surplus: spending into backing first shrinks the target, which leaves the surplus owed
     * to the LSP unchanged. A spend that exhausts the target eats the surplus directly.
     */
    fun spendConsumesLspSurplus(sc: StableChannel, price: Double, amountMsat: Long): Boolean {
        if (checkStabilityAction(sc, price).action != StabilityAction.PAY) return false
        val nativeSats = max(sc.stableReceiverBTC.sats - sc.backingSats, 0L)
        val overflowSats = amountMsat / 1000 - nativeSats
        if (overflowSats <= 0) return false
        val overflowUsd = overflowSats.toDouble() / Constants.SATS_IN_BTC * price
        return overflowUsd > sc.expectedUSD.amount
    }

    fun recomputeNative(sc: StableChannel) {
        val nativeSats = max(sc.stableReceiverBTC.sats - sc.backingSats, 0)
        sc.nativeChannelBTC = Bitcoin(nativeSats)
    }

    fun checkStabilityAction(sc: StableChannel, price: Double): StabilityCheckResult {
        val targetUSD = sc.expectedUSD.amount
        // A sub-cent target with no backing is a closed position. A sub-cent target WITH backing
        // is not: those sats are an unsettled LSP surplus (#322) and must settle like any other
        // above-par balance instead of being stranded by this bail.
        if ((targetUSD < MINIMUM_STABLE_USD && sc.backingSats == 0L) || price == 0.0) {
            return StabilityCheckResult(StabilityAction.STABLE, 0.0, 0.0, targetUSD, 0.0)
        }

        // No backing means no stable position — nothing to drift
        if (sc.backingSats == 0L) {
            return StabilityCheckResult(StabilityAction.STABLE, 0.0, 0.0, targetUSD, 0.0)
        }

        val stableUSDValue = (sc.backingSats.toDouble() / Constants.SATS_IN_BTC) * price

        val dollarsFromPar = stableUSDValue - targetUSD
        // Clamp the denominator: at a zero/tiny target an unclamped ratio is 0 (or explodes),
        // which would pin percentFromPar inside the deadband and block settlement of the residue.
        val percentFromPar = abs(dollarsFromPar / max(targetUSD, MINIMUM_STABLE_USD)) * 100.0

        val action =
            when {
                percentFromPar < Constants.STABILITY_THRESHOLD_PERCENT ||
                    abs(dollarsFromPar) < Constants.STABILITY_THRESHOLD_USD ->
                    StabilityAction.STABLE
                sc.riskLevel > Constants.MAX_RISK_LEVEL -> StabilityAction.HIGH_RISK_NO_ACTION
                sc.isStableReceiver && stableUSDValue < targetUSD -> StabilityAction.CHECK_ONLY
                else -> StabilityAction.PAY
            }

        return StabilityCheckResult(
            action,
            percentFromPar,
            stableUSDValue,
            targetUSD,
            dollarsFromPar,
        )
    }

    fun updateBalances(
        sc: StableChannel,
        channels: List<ChannelDetails>,
        onchainBalanceSats: Long,
        price: Double,
    ): StableChannel {
        val updated = sc.copy()
        updated.latestPrice = price
        updated.onchainBTC = Bitcoin(onchainBalanceSats)
        updated.onchainUSD = USD((onchainBalanceSats.toDouble() / Constants.SATS_IN_BTC) * price)

        // Find matching channel
        val channel =
            if (updated.userChannelId.isNotEmpty()) {
                channels.find { it.userChannelId == updated.userChannelId }
            } else {
                channels.firstOrNull()
            }

        if (channel == null) return updated

        // Auto-assign IDs if unset
        if (updated.userChannelId.isEmpty()) {
            updated.userChannelId = channel.userChannelId
        }
        if (updated.channelId.isEmpty() || updated.channelId != channel.channelId) {
            updated.channelId = channel.channelId
        }

        // Skip balance update if channel not ready (outbound=0 during pending)
        if (!channel.isChannelReady) return updated

        val ourBalanceSats =
            (channel.outboundCapacityMsat / 1000u).toLong() +
                (channel.unspendablePunishmentReserve?.toLong() ?: 0)
        val channelValueSats = channel.channelValueSats.toLong()
        val theirBalanceSats = channelValueSats - ourBalanceSats

        if (updated.isStableReceiver) {
            updated.stableReceiverBTC = Bitcoin(ourBalanceSats)
            updated.stableProviderBTC = Bitcoin(theirBalanceSats)
        } else {
            updated.stableReceiverBTC = Bitcoin(theirBalanceSats)
            updated.stableProviderBTC = Bitcoin(ourBalanceSats)
        }

        updated.stableReceiverUSD = USD.fromBitcoin(updated.stableReceiverBTC, price)
        updated.stableProviderUSD = USD.fromBitcoin(updated.stableProviderBTC, price)

        recomputeNative(updated)
        return updated
    }
}
