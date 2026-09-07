package com.stablechannels.app

import com.stablechannels.app.services.*
import com.stablechannels.app.models.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class StabilizationPolicyTest {
    // Mirrored verbatim from tests/fixtures/stabilization-limits.json (Rust and Swift share these).
    @Test fun canonicalVectors() {
        val vectors = listOf(
            StabilizationSnapshot(100000L, 100000L, 0L, 0.0, 100000.0) to 9795L,
            StabilizationSnapshot(200000L, 195000L, 50000L, 50.0, 100000.0) to 14295L,
            StabilizationSnapshot(1000001L, 1000001L, 500000L, 500.0, 100000.0) to 48999L,
            StabilizationSnapshot(200000L, 195000L, 50000L, 50.0, 80000.0) to 11000L,
            StabilizationSnapshot(2050L, 2050L, 0L, 0.0, 100000.0) to 0L,
            StabilizationSnapshot(100000L, 100000L, 99000L, 99.0, 100000.0) to 0L,
            StabilizationSnapshot(51984L, 51984L, 39810L, 25.407, 63304.4) to 640L,
            StabilizationSnapshot(51984L, 51984L, 39810L, 25.407, 63321.94) to 641L,
            StabilizationSnapshot(51984L, 51984L, 39810L, 25.407, 63425.91) to 642L,
            StabilizationSnapshot(150000L, 150000L, 100000L, 100.0, 100000.0) to 4795L
        )
        for ((snapshot, maximum) in vectors) {
            assertEquals(snapshot.toString(), maximum, snapshot.maxOrderCents())
            if (maximum > 0) assertTrue(snapshot.accepts(maximum))
            assertFalse(snapshot.accepts(maximum + 1))
            fun prepare(cents: Long): PreparedTrade? {
                val amount = cents / 100.0
                val fee = amount * 0.01
                val sc = StableChannel(channelId = "ab".repeat(32), userChannelId = "7",
                    expectedUSD = USD(snapshot.expectedUsd), stableReceiverBTC = Bitcoin(snapshot.receiverSats),
                    backingSats = snapshot.backingSats)
                return TradeProtocol.prepare(sc, snapshot.spendableSats, "sell", amount,
                    (amount - fee) / snapshot.price, fee, snapshot.expectedUsd + (amount - fee), snapshot.price)
            }
            if (maximum > 0) assertNotNull(snapshot.toString(), prepare(maximum))
            val above = try { prepare(maximum + 1) } catch (_: TradeValidationException) { null }
            assertNull(snapshot.toString(), above)
        }
    }

    @Test fun integerBoundaryAndSmallBalances() {
        assertEquals(990_000L, StabilizationPolicy.backingCap(1_000_001))
        assertEquals(98_000L, StabilizationPolicy.backingCap(100_000))
        assertNull(StabilizationPolicy.backingCap(1_999))
        assertNull(StabilizationPolicy.clientLimit(2_050))
        assertEquals(1L, StabilizationPolicy.clientLimit(2_051))
        assertTrue(StabilizationPolicy.backingCap(Long.MAX_VALUE)!! < Long.MAX_VALUE)
    }

    @Test fun changedSnapshotRejectsOldMaximum() {
        val initial = StabilizationSnapshot(200_000, 195_000, 50_000, 50.0, 100_000.0)
        assertFalse(initial.copy(spendableSats = 180_000).accepts(initial.maxOrderCents()))
    }

    @Test fun directPrepareCannotBypassLimitAndReductionsStayAllowed() {
        val sc = StableChannel(channelId = "ab".repeat(32), userChannelId = "7", expectedUSD = USD(99.0),
            stableReceiverBTC = Bitcoin(100_000), backingSats = 99_000)
        assertThrows(TradeValidationException::class.java) {
            TradeProtocol.prepare(sc, 100_000, "sell", 0.5, 0.00000495, 0.005, 99.495, 100_000.0)
        }
        assertNotNull(TradeProtocol.prepare(sc, 100_000, "buy", 1.0, 0.0000099, 0.01, 98.0, 100_000.0))
        assertNotNull(TradeProtocol.prepare(sc, 100_000, "buy", 99.0, 0.0009801, 0.99, 0.0, 100_000.0))
    }
}
