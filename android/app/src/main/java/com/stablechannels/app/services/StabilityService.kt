package com.stablechannels.app.services

import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.util.Constants
import org.lightningdevkit.ldknode.ChannelDetails
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToLong

object StabilityService {

    enum class StabilityAction(val value: String) {
        STABLE("STABLE"),
        HIGH_RISK_NO_ACTION("HIGH_RISK_NO_ACTION"),
        CHECK_ONLY("CHECK_ONLY"),
        PAY("PAY")
    }

    data class StabilityCheckResult(
        val action: StabilityAction,
        val percentFromPar: Double,
        val stableUSDValue: Double,
        val targetUSD: Double,
        val dollarsFromPar: Double
    )

    fun reconcileOutgoing(sc: StableChannel, price: Double): Pair<StableChannel, Double?> {
        val updated = sc.copy()
        if (updated.expectedUSD.amount < 0.01 || updated.backingSats == 0L || price == 0.0) {
            return Pair(updated, null)
        }
        if (updated.backingSats <= updated.stableReceiverBTC.sats) {
            return Pair(updated, null)
        }
        val overflowSats = updated.backingSats - updated.stableReceiverBTC.sats
        val usdToDeduct = (overflowSats.toDouble() / Constants.SATS_IN_BTC) * price
        val newExpected = max(updated.expectedUSD.amount - usdToDeduct, 0.0)
        updated.expectedUSD = USD(newExpected)
        updated.backingSats = ((newExpected / price) * Constants.SATS_IN_BTC).roundToLong()
        recomputeNative(updated)
        return Pair(updated, usdToDeduct)
    }

    fun reconcileIncoming(sc: StableChannel): StableChannel {
        val updated = sc.copy()
        if (updated.latestPrice > 0 && updated.expectedUSD.amount > 0) {
            updated.backingSats = ((updated.expectedUSD.amount / updated.latestPrice) * Constants.SATS_IN_BTC).roundToLong()
        }
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

    fun deductOutgoing(sc: StableChannel, amountSats: Long, price: Double): Double? {
        if (sc.expectedUSD.amount < 0.01 || price <= 0.0) return null
        val nativeSats = sc.nativeChannelBTC.sats
        if (amountSats <= nativeSats) return null  // Fully covered by native balance
        val overflowSats = amountSats - nativeSats
        val usdToDeduct = overflowSats.toDouble() / Constants.SATS_IN_BTC * price
        val newExpected = max(sc.expectedUSD.amount - usdToDeduct, 0.0)
        sc.expectedUSD = USD(newExpected)
        sc.backingSats = (newExpected / price * Constants.SATS_IN_BTC).toLong()
        recomputeNative(sc)
        return usdToDeduct
    }

    fun recomputeNative(sc: StableChannel) {
        val nativeSats = max(sc.stableReceiverBTC.sats - sc.backingSats, 0)
        sc.nativeChannelBTC = Bitcoin(nativeSats)
    }

    fun checkStabilityAction(sc: StableChannel, price: Double): StabilityCheckResult {
        val targetUSD = sc.expectedUSD.amount
        if (targetUSD < 0.01 || price == 0.0) {
            return StabilityCheckResult(StabilityAction.STABLE, 0.0, 0.0, targetUSD, 0.0)
        }

        val stableUSDValue = if (sc.backingSats > 0) {
            (sc.backingSats.toDouble() / Constants.SATS_IN_BTC) * price
        } else {
            sc.stableReceiverUSD.amount
        }

        val dollarsFromPar = stableUSDValue - targetUSD
        val percentFromPar = if (targetUSD > 0) abs(dollarsFromPar / targetUSD) * 100.0 else 0.0

        val action = when {
            percentFromPar < Constants.STABILITY_THRESHOLD_PERCENT
                || abs(dollarsFromPar) < Constants.STABILITY_THRESHOLD_USD -> StabilityAction.STABLE
            sc.riskLevel > Constants.MAX_RISK_LEVEL -> StabilityAction.HIGH_RISK_NO_ACTION
            sc.isStableReceiver && stableUSDValue < targetUSD -> StabilityAction.CHECK_ONLY
            else -> StabilityAction.PAY
        }

        return StabilityCheckResult(action, percentFromPar, stableUSDValue, targetUSD, dollarsFromPar)
    }

    fun updateBalances(
        sc: StableChannel,
        channels: List<ChannelDetails>,
        onchainBalanceSats: Long,
        price: Double
    ): StableChannel {
        val updated = sc.copy()
        updated.latestPrice = price
        updated.onchainBTC = Bitcoin(onchainBalanceSats)
        updated.onchainUSD = USD((onchainBalanceSats.toDouble() / Constants.SATS_IN_BTC) * price)

        // Find matching channel
        val channel = if (updated.userChannelId.isNotEmpty()) {
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

        val ourBalanceSats = (channel.outboundCapacityMsat / 1000u).toLong() +
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
