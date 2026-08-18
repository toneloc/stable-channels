package com.stablechannels.app

import com.stablechannels.app.util.NamedPrice
import com.stablechannels.app.util.PriceOracle
import com.stablechannels.app.util.PriceOracleAnchorStore
import com.stablechannels.app.util.PriceOracleException
import com.stablechannels.app.util.PriceOracleSource
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class PriceOracleTest {
    private fun named(values: List<Double>, prefix: String = "feed") =
        values.mapIndexed { index, value -> NamedPrice("$prefix-$index", value) }

    @Test
    fun `direct USD consensus is preferred`() {
        val result = PriceOracle.resolve(
            named(listOf(64_000.0, 64_050.0, 63_950.0)),
            named(listOf(80_000.0, 80_100.0, 79_900.0), "usdt"),
            named(listOf(1.0, 1.0, 1.0), "peg"),
            64_000.0
        )

        assertEquals(PriceOracleSource.DIRECT_USD, result.source)
        assertEquals(64_000.0, result.price, 0.001)
        assertNull(result.usdtUsd)
    }

    @Test
    fun `USDT fallback uses measured peg`() {
        val result = PriceOracle.resolve(
            named(listOf(64_000.0, 64_050.0)),
            named(listOf(64_064.0, 64_074.0, 64_054.0), "usdt"),
            named(listOf(0.999, 0.9991, 0.9989), "peg"),
            64_000.0
        )

        assertEquals(PriceOracleSource.NORMALIZED_USDT, result.source)
        assertEquals(0.999, result.usdtUsd!!, 0.000001)
        assertEquals(63_999.936, result.price, 1.0)
    }

    @Test
    fun `USDT fallback rejects depeg`() {
        assertThrows(PriceOracleException::class.java) {
            PriceOracle.resolve(
                named(listOf(64_000.0)),
                named(listOf(64_500.0, 64_520.0, 64_480.0), "usdt"),
                named(listOf(0.98, 0.981, 0.979), "peg"),
                64_000.0
            )
        }
    }

    @Test
    fun `large move quarantines prior price`() {
        val error = assertThrows(PriceOracleException::class.java) {
            PriceOracle.validateBitcoinConsensus(
                named(listOf(80_000.0, 80_100.0, 79_900.0)),
                64_000.0
            )
        }
        assertTrue(error.quarantinesPrice)
    }

    @Test
    fun `primary list contains only six direct USD books`() {
        assertEquals(6, PriceOracle.DIRECT_USD_FEEDS.size)
        assertTrue(PriceOracle.DIRECT_USD_FEEDS.none { it.urlFormat.uppercase().contains("USDT") })
    }

    @Test
    fun `peg gate survives direct USD host outage`() {
        // The USDT fallback's peg gate needs MINIMUM_AGREEING_PEG_FEEDS. If too many peg feeds
        // share hosts with the direct-USD tier, the fallback fails exactly when the primary
        // tier is unreachable — the outage it exists to survive.
        fun host(url: String) = url.substringAfter("//").substringBefore("/")
        val usdHosts = PriceOracle.DIRECT_USD_FEEDS.map { host(it.urlFormat) }.toSet()
        val disjoint = PriceOracle.USDT_USD_FEEDS.count { host(it.urlFormat) !in usdHosts }
        assertTrue(disjoint >= PriceOracle.MINIMUM_AGREEING_PEG_FEEDS)
    }

    @Test
    fun `fresh anchor protects the background service`() {
        val nowMs = 1_000_000L
        assertEquals(
            64_000.0,
            PriceOracleAnchorStore.freshPrice(64_000.0, nowMs - 60_000, nowMs)!!,
            0.0
        )
    }

    @Test
    fun `stale future or implausible anchor is rejected`() {
        val nowMs = 1_000_000L
        assertNull(PriceOracleAnchorStore.freshPrice(64_000.0, nowMs - 61_000, nowMs))
        assertNull(PriceOracleAnchorStore.freshPrice(64_000.0, nowMs + 1_000, nowMs))
        assertNull(PriceOracleAnchorStore.freshPrice(0.0, nowMs - 1_000, nowMs))
    }
}
