package com.stablechannels.app

import org.lightningdevkit.ldknode.BalanceDetails
import org.lightningdevkit.ldknode.ChannelConfig
import org.lightningdevkit.ldknode.ChannelDetails
import org.lightningdevkit.ldknode.MaxDustHtlcExposure
import org.lightningdevkit.ldknode.NoPointer
import org.lightningdevkit.ldknode.Node
import org.lightningdevkit.ldknode.PaymentDetails
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements

// UniFFI uses Android's cleaner even for a NoPointer transport. Supply a JVM cleaner
// so these tests need neither a native node nor JDK module-export flags.
@Implements(className = "android.system.SystemCleaner", isInAndroidSdk = false)
class SystemCleanerShadow {
    companion object {
        @JvmStatic @Implementation
        fun cleaner(): java.lang.ref.Cleaner = java.lang.ref.Cleaner.create()
    }
}

/** Only the LDK transport is substituted; channels and payments are whatever a test sets. */
internal class TestNode : Node(NoPointer) {
    var channels = emptyList<ChannelDetails>()
    var payments = emptyList<PaymentDetails>()
    var channelSnapshots = mutableListOf<List<ChannelDetails>>()
    override fun listChannels(): List<ChannelDetails> {
        if (channelSnapshots.isNotEmpty()) return channelSnapshots.removeAt(0)
        return channels
    }
    override fun listPayments() = payments
    override fun payment(paymentId: String) = payments.singleOrNull { it.id == paymentId }
    override fun listBalances() = BalanceDetails(0uL, 0uL, 0uL,
        channels.sumOf { it.outboundCapacityMsat / 1000uL }, emptyList(), emptyList())
}

internal fun channel(uid: String = "7", cid: String = "channel", receiver: Long = 20_000, ready: Boolean = true) = ChannelDetails(
    channelId = cid, counterpartyNodeId = "peer", fundingTxo = null, fundingRedeemScript = null,
    shortChannelId = null, outboundScidAlias = null, inboundScidAlias = null,
    channelValueSats = 50_000uL, unspendablePunishmentReserve = 0uL, userChannelId = uid,
    feerateSatPer1000Weight = 0u, outboundCapacityMsat = receiver.toULong() * 1000uL,
    inboundCapacityMsat = 0uL, confirmationsRequired = null, confirmations = null,
    isOutbound = true, isChannelReady = ready, isUsable = ready, isAnnounced = false,
    cltvExpiryDelta = null, counterpartyUnspendablePunishmentReserve = 0uL,
    counterpartyOutboundHtlcMinimumMsat = null, counterpartyOutboundHtlcMaximumMsat = null,
    counterpartyForwardingInfoFeeBaseMsat = null, counterpartyForwardingInfoFeeProportionalMillionths = null,
    counterpartyForwardingInfoCltvExpiryDelta = null, nextOutboundHtlcLimitMsat = 0uL,
    nextOutboundHtlcMinimumMsat = 0uL, forceCloseSpendDelay = null, inboundHtlcMinimumMsat = 0uL,
    inboundHtlcMaximumMsat = null, config = ChannelConfig(0u, 0u, 0u, MaxDustHtlcExposure.FixedLimit(0uL), 0uL, false), channelShutdownState = null
)
