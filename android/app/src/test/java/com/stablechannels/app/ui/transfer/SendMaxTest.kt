package com.stablechannels.app.ui.transfer

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SendMaxTest {
    @Test
    fun maxLeavesRoomForTheRoutingFeeCeiling() {
        // 9,900 + 99 + 50 = 10,049; one sat more needs a 100 sat ceiling and goes over.
        assertEquals(9_900L, SendMax.lightningSats(10_050, null))
        for (balance in listOf(51L, 151L, 10_000L, 44_516L, 1_000_000L)) {
            val max = SendMax.lightningSats(balance, null)
            assertTrue(max + SendMax.maxRoutingFeeSats(max) <= balance)
            assertTrue(max + 1 + SendMax.maxRoutingFeeSats(max + 1) > balance)
        }
    }

    @Test
    fun usesTheWholeBalanceNotJustTheStablePart() {
        // 44,516 sats in the channel, whatever share of it is stable.
        assertEquals(44_025L, SendMax.lightningSats(44_516, 50_000))
    }

    @Test
    fun outboundCapacityCapsTheMax() {
        assertEquals(SendMax.lightningSats(30_000, null), SendMax.lightningSats(44_516, 30_000))
    }

    @Test
    fun tooLittleForTheFeeGivesZero() {
        assertEquals(0L, SendMax.lightningSats(50, null))
        assertEquals(0L, SendMax.lightningSats(0, null))
    }

    @Test
    fun spliceOutMaxLeavesTheSpliceFee() {
        // 2 sat/vB on 180 vB.
        assertEquals(29_640L, SendMax.spliceOutSats(44_516, 30_000, 2))
        assertEquals(44_156L, SendMax.spliceOutSats(44_516, 50_000, 2))
        assertEquals(0L, SendMax.spliceOutSats(300, 300, 2))
    }

    @Test
    fun usdIsRoundedDown() {
        // 44,025 sats at $100,000 is $44.025.
        assertEquals(44.02, SendMax.usd(44_025, 100_000.0), 0.0)
    }
}
