package com.stablechannels.app.domain.send

import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SendChannelSpendPolicyTest {
    private fun channel(id: String, outboundSats: Long, isReady: Boolean = true) =
        SpliceChannelCandidate(
            channelId = id,
            userChannelId = id,
            counterpartyNodeId =
                "020202020202020202020202020202020202020202020202020202020202020202",
            outboundCapacityMsat = outboundSats * 1_000,
            isReady = isReady,
        )

    private val onchain =
        SendDestination.Onchain("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", null)

    @Test
    fun `lightning aggregates ready channels`() {
        val spendable =
            SendChannelSpendPolicy.availableSpendableSats(
                destination = SendDestination.LnurlPay(URI("https://example.com")),
                channels = listOf(channel("ch1", 30_000), channel("ch2", 50_000)),
                lightningBalanceSats = 100_000,
                onchainBalanceSats = 10_000,
                totalBalanceSats = 110_000,
                isSweeping = false,
            )

        // Lightning aggregates all ready channels: 30k + 50k = 80k
        assertEquals(80_000L, spendable)
    }

    @Test
    fun `onchain splice caps to largest single channel`() {
        val spendable =
            SendChannelSpendPolicy.availableSpendableSats(
                destination = onchain,
                channels = listOf(channel("ch1", 30_000), channel("ch2", 50_000)),
                lightningBalanceSats = 100_000,
                onchainBalanceSats = 10_000,
                totalBalanceSats = 110_000,
                isSweeping = false,
            )

        assertEquals(50_000L, spendable)
    }

    @Test
    fun `onchain with no ready channels uses onchain balance`() {
        val spendable =
            SendChannelSpendPolicy.availableSpendableSats(
                destination = onchain,
                channels = emptyList(),
                lightningBalanceSats = 100_000,
                onchainBalanceSats = 15_000,
                totalBalanceSats = 115_000,
                isSweeping = false,
            )

        assertEquals(15_000L, spendable)
    }

    @Test
    fun `select splice channel picks best fit`() {
        val selected =
            SendChannelSpendPolicy.selectSpliceChannel(
                channels =
                    listOf(
                        channel("small", 20_000),
                        channel("large", 80_000),
                        channel("medium", 45_000),
                    ),
                amountSats = 40_000,
                feeRateSatVb = 0.0,
            )

        assertEquals("medium", selected?.userChannelId)
    }

    @Test
    fun `select splice channel skips unready channels`() {
        val selected =
            SendChannelSpendPolicy.selectSpliceChannel(
                channels =
                    listOf(channel("unready", 90_000, isReady = false), channel("ready", 30_000)),
                amountSats = 25_000,
                feeRateSatVb = 0.0,
            )

        assertEquals("ready", selected?.userChannelId)
    }

    @Test
    fun `select splice channel falls back to largest when none sufficient`() {
        val selected =
            SendChannelSpendPolicy.selectSpliceChannel(
                channels = listOf(channel("ch1", 20_000), channel("ch2", 35_000)),
                amountSats = 50_000,
                feeRateSatVb = 0.0,
            )

        assertEquals("ch2", selected?.userChannelId)
    }

    @Test
    fun `select splice channel exact capacity match is sufficient`() {
        val selected =
            SendChannelSpendPolicy.selectSpliceChannel(
                channels =
                    listOf(
                        channel("small", 20_000),
                        channel("large", 80_000),
                        channel("exact", 40_000),
                    ),
                amountSats = 40_000,
                feeRateSatVb = 0.0,
            )

        assertEquals("exact", selected?.userChannelId)
    }

    @Test
    fun `select splice channel requires fee headroom`() {
        val amount = 40_000L
        val fee = PaymentFeeEstimator.estimateSpliceOutFee(10.0)
        val selected =
            SendChannelSpendPolicy.selectSpliceChannel(
                channels =
                    listOf(
                        channel("short", amount + fee - 1),
                        channel("large", amount + fee + 10_000),
                    ),
                amountSats = amount,
                feeRateSatVb = 10.0,
            )

        assertEquals(
            "A channel one sat short of amount + fee cannot fund the splice",
            "large",
            selected?.userChannelId,
        )
    }

    @Test
    fun `select splice channel exact amount plus fee is best fit`() {
        val amount = 40_000L
        val fee = PaymentFeeEstimator.estimateSpliceOutFee(10.0)
        val selected =
            SendChannelSpendPolicy.selectSpliceChannel(
                channels =
                    listOf(channel("large", amount + fee + 10_000), channel("exact", amount + fee)),
                amountSats = amount,
                feeRateSatVb = 10.0,
            )

        assertEquals("exact", selected?.userChannelId)
    }

    @Test
    fun `onchain spendable splice out is bounded by largest channel`() {
        val channels = listOf(channel("a", 30_000), channel("b", 50_000))

        assertEquals(
            "A splice-out draws on one channel, not the 80,000 sat total",
            50_000L,
            SendChannelSpendPolicy.availableOnchainSpendableSats(channels, 80_000, 7_000, false),
        )
        assertEquals(
            7_000L,
            SendChannelSpendPolicy.availableOnchainSpendableSats(channels, 80_000, 7_000, true),
        )
        assertEquals(
            7_000L,
            SendChannelSpendPolicy.availableOnchainSpendableSats(emptyList(), 80_000, 7_000, false),
        )
    }

    @Test
    fun `max sendable principal cap bounds amount not fee`() {
        val fee: (Long) -> Long = { it / 100 + 50 }
        // Cap fits with its fee inside the balance: the cap itself.
        assertEquals(
            50_000L,
            SendChannelSpendPolicy.calculateMaxSendable(100_000, 50_000, fee),
        )
        // Balance binds first: the fee-inclusive max, unaffected by the cap.
        assertEquals(
            SendChannelSpendPolicy.calculateMaxSendable(30_000, feeEstimator = fee),
            SendChannelSpendPolicy.calculateMaxSendable(30_000, 50_000, fee),
        )
    }

    @Test
    fun `onchain send all only for max onchain without splice out`() {
        val ready = listOf(channel("a", 50_000))

        assertTrue(SendChannelSpendPolicy.isOnchainSendAll(true, onchain, emptyList(), false))
        assertFalse(SendChannelSpendPolicy.isOnchainSendAll(false, onchain, emptyList(), false))
        assertFalse(
            "A splice-out Max draws on one channel",
            SendChannelSpendPolicy.isOnchainSendAll(true, onchain, ready, false),
        )
        assertTrue(SendChannelSpendPolicy.isOnchainSendAll(true, onchain, ready, true))
        assertFalse(SendChannelSpendPolicy.isOnchainSendAll(true, null, emptyList(), false))
    }
}
