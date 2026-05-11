package com.stablechannels.app

import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.PriceService
import com.stablechannels.app.services.StabilityService
import com.stablechannels.app.services.StabilityService.StabilityAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class StabilityServiceTest {

    // ---------------------------------------------------------------------------
    // checkStabilityAction — core stability logic
    // ---------------------------------------------------------------------------

    @Test
    fun `returns STABLE when no expected USD set`() {
        val sc = StableChannel()  // expectedUSD defaults to 0
        val result = StabilityService.checkStabilityAction(sc, price = 85_000.0)
        assertEquals(StabilityAction.STABLE, result.action)
    }

    @Test
    fun `returns STABLE when price is zero`() {
        val sc = StableChannel(expectedUSD = USD(100.0), backingSats = 117_647L)
        val result = StabilityService.checkStabilityAction(sc, price = 0.0)
        assertEquals(StabilityAction.STABLE, result.action)
    }

    @Test
    fun `returns STABLE when within 10 cent threshold`() {
        // $100 target, backing sats = 100/85000 * 1e8 = 117_647 sats
        val backingSats = (100.0 / 85_000.0 * 100_000_000).toLong()
        val sc = StableChannel(
            expectedUSD = USD(100.0),
            backingSats = backingSats,
            isStableReceiver = true
        )
        val result = StabilityService.checkStabilityAction(sc, price = 85_000.0)
        assertEquals(StabilityAction.STABLE, result.action)
        assertEquals(0.0, result.percentFromPar, 0.01)
    }

    @Test
    fun `returns PAY when BTC price rises and receiver has excess`() {
        // $100 target. If price rises to 90_000, backing sats are worth MORE.
        // Receiver (provider side here) owes sats back.
        val backingSats = (100.0 / 85_000.0 * 100_000_000).toLong()
        val sc = StableChannel(
            expectedUSD = USD(100.0),
            backingSats = backingSats,
            isStableReceiver = false   // provider pays when price rises
        )
        val result = StabilityService.checkStabilityAction(sc, price = 90_000.0)
        assertEquals(StabilityAction.PAY, result.action)
        assertTrue(result.dollarsFromPar > 0)
    }

    @Test
    fun `returns CHECK_ONLY when receiver is below peg`() {
        // Price dropped — receiver's backing sats worth LESS than target
        val backingSats = (100.0 / 85_000.0 * 100_000_000).toLong()
        val sc = StableChannel(
            expectedUSD = USD(100.0),
            backingSats = backingSats,
            isStableReceiver = true
        )
        // Price drops to 80_000 — backing_sats now worth only ~$94
        val result = StabilityService.checkStabilityAction(sc, price = 80_000.0)
        // Receiver checks only — provider should pay, not receiver
        assertEquals(StabilityAction.CHECK_ONLY, result.action)
    }

    @Test
    fun `returns HIGH_RISK_NO_ACTION when riskLevel exceeds max`() {
        val backingSats = (100.0 / 85_000.0 * 100_000_000).toLong()
        val sc = StableChannel(
            expectedUSD = USD(100.0),
            backingSats = backingSats,
            riskLevel = 101,           // exceeds MAX_RISK_LEVEL = 100
            isStableReceiver = false
        )
        val result = StabilityService.checkStabilityAction(sc, price = 90_000.0)
        assertEquals(StabilityAction.HIGH_RISK_NO_ACTION, result.action)
    }

    @Test
    fun `percentFromPar is calculated correctly`() {
        // Target: $100. Backing sats worth $110 at new price.
        val backingSats = (100.0 / 85_000.0 * 100_000_000).toLong()
        val sc = StableChannel(
            expectedUSD = USD(100.0),
            backingSats = backingSats,
            isStableReceiver = false
        )
        val newPrice = 85_000.0 * 1.10  // 10% price increase → backing worth $110
        val result = StabilityService.checkStabilityAction(sc, price = newPrice)
        assertEquals(10.0, result.percentFromPar, 0.5)
    }

    // ---------------------------------------------------------------------------
    // applyTrade
    // ---------------------------------------------------------------------------

    @Test
    fun `applyTrade updates expectedUSD and backingSats`() {
        val sc = StableChannel(expectedUSD = USD(100.0), backingSats = 117_647L)
        val price = 85_000.0
        val newExpected = 150.0

        val updated = StabilityService.applyTrade(sc, newExpected, price)

        assertEquals(150.0, updated.expectedUSD.amount, 0.01)
        val expectedSats = (150.0 / price * 100_000_000).toLong()
        // Allow ±1 sat tolerance for floating-point rounding in the conversion
        assertTrue(kotlin.math.abs(updated.backingSats - expectedSats) <= 1)
    }

    @Test
    fun `applyTrade does not modify backingSats when price is zero`() {
        val sc = StableChannel(expectedUSD = USD(100.0), backingSats = 117_647L)
        val updated = StabilityService.applyTrade(sc, 200.0, price = 0.0)
        // backingSats unchanged when price = 0
        assertEquals(117_647L, updated.backingSats)
    }

    // ---------------------------------------------------------------------------
    // reconcileOutgoing
    // ---------------------------------------------------------------------------

    @Test
    fun `reconcileOutgoing returns unchanged when no stable position`() {
        val sc = StableChannel()  // expectedUSD = 0
        val (updated, deducted) = StabilityService.reconcileOutgoing(sc, price = 85_000.0)
        assertEquals(null, deducted)
        assertEquals(0.0, updated.expectedUSD.amount, 0.0)
    }

    @Test
    fun `reconcileOutgoing deducts overflow sats from expectedUSD`() {
        val price = 85_000.0
        val expectedSats = (100.0 / price * 100_000_000).toLong()
        val sc = StableChannel(
            expectedUSD = USD(100.0),
            backingSats = expectedSats + 50_000L,   // 50k extra sats
            stableReceiverBTC = Bitcoin(expectedSats)
        )
        val (_, deducted) = StabilityService.reconcileOutgoing(sc, price)
        val expectedDeduction = 50_000.0 / 100_000_000.0 * price
        assertEquals(expectedDeduction, deducted!!, 0.01)
    }

    // ---------------------------------------------------------------------------
    // PriceService.median — pure math
    // ---------------------------------------------------------------------------

    @Test
    fun `median of empty list is zero`() {
        assertEquals(0.0, PriceService.median(emptyList()), 0.0)
    }

    @Test
    fun `median of single value is itself`() {
        assertEquals(42.0, PriceService.median(listOf(42.0)), 0.0)
    }

    @Test
    fun `median of odd list`() {
        assertEquals(3.0, PriceService.median(listOf(1.0, 3.0, 5.0)), 0.0)
    }

    @Test
    fun `median of even list averages middle two`() {
        assertEquals(2.5, PriceService.median(listOf(1.0, 2.0, 3.0, 4.0)), 0.0)
    }

    @Test
    fun `median ignores order of input`() {
        assertEquals(3.0, PriceService.median(listOf(5.0, 1.0, 3.0)), 0.0)
    }

    @Test
    fun `median with realistic BTC prices`() {
        val prices = listOf(84_500.0, 85_000.0, 85_100.0, 84_900.0, 85_050.0)
        assertEquals(85_000.0, PriceService.median(prices), 0.0)
    }

    // ---------------------------------------------------------------------------
    // USD model helpers
    // ---------------------------------------------------------------------------

    @Test
    fun `USD toMsats converts correctly`() {
        val usd = USD(85.0)
        val price = 85_000.0
        // $85 / $85000 per BTC = 0.001 BTC = 100_000 sats = 100_000_000 msats
        val expected = 100_000_000L
        assertEquals(expected, usd.toMsats(price))
    }

    @Test
    fun `Bitcoin fromBTC round trips correctly`() {
        val btc = Bitcoin.fromBTC(0.001)
        assertEquals(100_000L, btc.sats)
    }
}
