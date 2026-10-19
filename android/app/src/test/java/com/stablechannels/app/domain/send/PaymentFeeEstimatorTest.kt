package com.stablechannels.app.domain.send

import com.stablechannels.app.util.Constants
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PaymentFeeEstimatorTest {
    private val delta = 0.001
    private val floor = Constants.MIN_FEE_RATE_SAT_VB
    private val ceiling = Constants.MAX_FEE_RATE_SAT_VB

    // Lightning routing fee ceiling

    @Test
    fun `max routing fee msat is one percent plus fifty sats`() {
        assertEquals(1_050_000L, PaymentFeeEstimator.maxLightningRoutingFeeMsat(100_000_000))
    }

    @Test
    fun `max routing fee msat of zero amount is base only`() {
        assertEquals(50_000L, PaymentFeeEstimator.maxLightningRoutingFeeMsat(0))
    }

    @Test
    fun `max routing fee msat of max amount does not overflow`() {
        // Long.MAX_VALUE / 100 + 50_000 fits without saturating.
        assertEquals(
            92_233_720_368_597_758L,
            PaymentFeeEstimator.maxLightningRoutingFeeMsat(Long.MAX_VALUE),
        )
    }

    @Test
    fun `max routing fee sats of zero sats is zero`() {
        assertEquals(0L, PaymentFeeEstimator.maxLightningRoutingFeeSats(0))
    }

    @Test
    fun `max routing fee sats is whole sat ceiling`() {
        assertEquals(1_050L, PaymentFeeEstimator.maxLightningRoutingFeeSats(100_000))
    }

    @Test
    fun `max routing fee sats rounds up partial sat`() {
        // 1_000 msat / 100 + 50_000 = 50_010 msat -> 51 sats
        assertEquals(51L, PaymentFeeEstimator.maxLightningRoutingFeeSats(1))
    }

    @Test
    fun `max routing fee sats of max sats saturates without overflow`() {
        // sats * 1_000 saturates to Long.MAX_VALUE msat; cap 92_233_720_368_597_758 msat, rounded
        // up.
        assertEquals(
            92_233_720_368_598L,
            PaymentFeeEstimator.maxLightningRoutingFeeSats(Long.MAX_VALUE),
        )
    }

    @Test
    fun `max routing fee for exact msat rounds up sub sat remainder`() {
        // 100,000.999 sats: cap is 1,050,009 msat, which must show as 1,051 sats, not 1,050.
        assertEquals(1_051L, PaymentFeeEstimator.maxLightningRoutingFeeSatsForMsat(100_000_999))
        assertEquals(1_050L, PaymentFeeEstimator.maxLightningRoutingFeeSatsForMsat(100_000_000))
        assertEquals(0L, PaymentFeeEstimator.maxLightningRoutingFeeSatsForMsat(0))
    }

    @Test
    fun `max lightning debit covers amount plus ceiling rounded up`() {
        // 100,000,999 + 1,050,009 = 101,051,008 msat -> 101,052 sats.
        assertEquals(101_052L, PaymentFeeEstimator.maxLightningDebitSats(100_000_999))
        assertEquals(101_050L, PaymentFeeEstimator.maxLightningDebitSats(100_000_000))
        assertEquals(0L, PaymentFeeEstimator.maxLightningDebitSats(0))
        assertEquals(
            Long.MAX_VALUE / 1000 + 1,
            PaymentFeeEstimator.maxLightningDebitSats(Long.MAX_VALUE),
        )
    }

    // On-chain fee estimation

    @Test
    fun `standard send uses estimated vbytes`() {
        assertEquals(
            10 * Constants.ESTIMATED_ONCHAIN_SEND_VBYTES,
            PaymentFeeEstimator.estimateOnchainFee(10.0, isSendAll = false),
        )
    }

    @Test
    fun `send all uses send all vbytes`() {
        assertEquals(
            10 * Constants.ESTIMATED_ONCHAIN_SEND_ALL_VBYTES,
            PaymentFeeEstimator.estimateOnchainFee(10.0, isSendAll = true),
        )
    }

    @Test
    fun `splice out fee uses splice vbytes`() {
        assertEquals(
            10 * Constants.ESTIMATED_SPLICE_OUT_VBYTES,
            PaymentFeeEstimator.estimateSpliceOutFee(10.0),
        )
    }

    @Test
    fun `custom vbytes`() {
        assertEquals(
            3_000L,
            PaymentFeeEstimator.estimateOnchainFee(12.0, isSendAll = false, sendVBytes = 250),
        )
    }

    @Test
    fun `fractional rate rounds up`() {
        // 1.12 sat/vB * 250 vB = 280 sat
        assertEquals(
            280L,
            PaymentFeeEstimator.estimateOnchainFee(1.12, isSendAll = false, sendVBytes = 250),
        )
    }

    @Test
    fun `sat per kwu keeps fractional rate and never rounds below it`() {
        assertEquals(250L, PaymentFeeEstimator.satPerKwu(1.0))
        assertEquals(320L, PaymentFeeEstimator.satPerKwu(1.28))
        assertEquals(450L, PaymentFeeEstimator.satPerKwu(1.8))
        assertEquals(251L, PaymentFeeEstimator.satPerKwu(1.001))
        assertEquals(2_500_000L, PaymentFeeEstimator.satPerKwu(ceiling))
    }

    @Test
    fun `huge fee rate saturates`() {
        assertEquals(
            Long.MAX_VALUE,
            PaymentFeeEstimator.estimateOnchainFee(Double.MAX_VALUE, isSendAll = false),
        )
    }

    @Test
    fun `non finite or non positive rate returns zero`() {
        for (rate in listOf(-5.0, 0.0, Double.NaN, Double.POSITIVE_INFINITY)) {
            assertEquals(
                "rate $rate",
                0L,
                PaymentFeeEstimator.estimateOnchainFee(rate, isSendAll = false),
            )
        }
    }

    @Test
    fun `splice out vbyte estimate is pinned`() {
        // Pinned to a literal: other tests derive their expectation from this constant.
        assertEquals(180L, Constants.ESTIMATED_SPLICE_OUT_VBYTES)
    }

    // Network fee speed tier

    @Test
    fun `tier standard baseline`() {
        assertEquals(8.0, NetworkFeeSpeedTier.ECONOMY.effectiveRate(10.0), delta)
        assertEquals(10.0, NetworkFeeSpeedTier.STANDARD.effectiveRate(10.0), delta)
        assertEquals(13.0, NetworkFeeSpeedTier.PRIORITY.effectiveRate(10.0), delta)
    }

    @Test
    fun `tier low fee mempool`() {
        assertEquals(floor, NetworkFeeSpeedTier.ECONOMY.effectiveRate(1.0), delta)
        assertEquals(1.0, NetworkFeeSpeedTier.STANDARD.effectiveRate(1.0), delta)
        assertEquals(1.3, NetworkFeeSpeedTier.PRIORITY.effectiveRate(1.0), delta)
    }

    @Test
    fun `tier congested mempool`() {
        assertEquals(40.0, NetworkFeeSpeedTier.ECONOMY.effectiveRate(50.0), delta)
        assertEquals(50.0, NetworkFeeSpeedTier.STANDARD.effectiveRate(50.0), delta)
        assertEquals(65.0, NetworkFeeSpeedTier.PRIORITY.effectiveRate(50.0), delta)
    }

    @Test
    fun `tier uses recommended fees when present`() {
        val rec = RecommendedFees(25.0, 18.0, 12.0, minimumFee = 2.0)
        assertEquals(25.0, NetworkFeeSpeedTier.PRIORITY.effectiveRate(10.0, rec), delta)
        assertEquals(18.0, NetworkFeeSpeedTier.STANDARD.effectiveRate(10.0, rec), delta)
        assertEquals(12.0, NetworkFeeSpeedTier.ECONOMY.effectiveRate(10.0, rec), delta)
    }

    @Test
    fun `tier rates are monotonic across range`() {
        for (rateInt in 1..200) {
            val rate = rateInt.toDouble()
            val eco = NetworkFeeSpeedTier.ECONOMY.effectiveRate(rate)
            val std = NetworkFeeSpeedTier.STANDARD.effectiveRate(rate)
            val pri = NetworkFeeSpeedTier.PRIORITY.effectiveRate(rate)
            assertTrue("economy below floor at $rate", eco >= floor)
            assertTrue("economy above standard at $rate", eco <= std)
            assertTrue("priority not above standard at $rate", pri > std)
        }
    }

    @Test
    fun `recommended fees clamp inverted rates to monotonic`() {
        // Inverted rates: hourFee (20) > halfHour (10) > fastest (5)
        val inverted = RecommendedFees(5.0, 10.0, 20.0, minimumFee = 2.0)
        val pri = inverted.rate(NetworkFeeSpeedTier.PRIORITY)
        val std = inverted.rate(NetworkFeeSpeedTier.STANDARD)
        val eco = inverted.rate(NetworkFeeSpeedTier.ECONOMY)
        assertTrue("priority must clamp to at least standard", pri >= std)
        assertTrue("standard must clamp to at least economy", std >= eco)
        assertTrue("economy must respect minimum fee", eco >= 2.0)
    }

    @Test
    fun `recommended fees quiet mempool all one`() {
        val quiet = RecommendedFees(1.0, 1.0, 1.0, economyFee = 1.0, minimumFee = 1.0)
        for (tier in NetworkFeeSpeedTier.entries) {
            assertEquals(tier.name, 1.0, quiet.rate(tier), delta)
        }
    }

    @Test
    fun `recommended fees sub floor rates clamp to floor`() {
        val subFloor = RecommendedFees(0.5, 0.5, 0.5, economyFee = 0.5, minimumFee = 0.5)
        for (tier in NetworkFeeSpeedTier.entries) {
            assertTrue("$tier below floor", subFloor.rate(tier) >= floor)
            assertTrue("$tier below floor", tier.effectiveRate(0.5, subFloor) >= floor)
        }
    }

    @Test
    fun `sub floor base rate without recommended fees stays at floor`() {
        for (tier in NetworkFeeSpeedTier.entries) {
            assertTrue("$tier below floor", tier.effectiveRate(0.3) >= floor)
        }
        assertTrue(
            "priority must strictly exceed standard at the floor",
            NetworkFeeSpeedTier.PRIORITY.effectiveRate(0.3) >
                NetworkFeeSpeedTier.STANDARD.effectiveRate(0.3),
        )
    }

    @Test
    fun `zero negative and nan base rate stay within bounds`() {
        for (base in listOf(0.0, -5.0, Double.NaN)) {
            assertEquals(floor, NetworkFeeSpeedTier.STANDARD.effectiveRate(base), 0.0)
            for (tier in NetworkFeeSpeedTier.entries) {
                val rate = tier.effectiveRate(base)
                assertTrue("$tier at base $base", rate >= floor)
                assertTrue("$tier at base $base", rate <= ceiling)
            }
        }
    }

    @Test
    fun `above ceiling clamps to ceiling`() {
        val huge = RecommendedFees(20_000.0, 20_000.0, 20_000.0, 20_000.0, 20_000.0)
        for (tier in NetworkFeeSpeedTier.entries) {
            assertEquals(tier.name, ceiling, tier.effectiveRate(20_000.0, huge), 0.0)
        }
        // Without recommended fees, standard and priority hit the ceiling; economy stays below it.
        assertEquals(ceiling, NetworkFeeSpeedTier.STANDARD.effectiveRate(20_000.0), 0.0)
        assertEquals(ceiling, NetworkFeeSpeedTier.PRIORITY.effectiveRate(20_000.0), 0.0)
        assertTrue(NetworkFeeSpeedTier.ECONOMY.effectiveRate(20_000.0) <= ceiling)
    }
}
