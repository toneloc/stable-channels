package com.stablechannels.app.domain.send

import java.math.BigDecimal
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SendAmountCalculatorTest {

    private fun effective(text: String, unit: SendAmountUnit, btcPrice: Double): Long =
        SendAmountCalculator.effectiveSats(null, text, unit, btcPrice)

    @Test
    fun `effective sats across units`() {
        // $65.00 at $65,000/BTC = 0.001 BTC = 100,000 sats
        assertEquals(100_000L, effective("65.00", SendAmountUnit.USD, 65_000.0))
        assertEquals(50_000L, effective("50000", SendAmountUnit.SATS, 65_000.0))
        assertEquals(100_000L, effective("0.001", SendAmountUnit.BTC, 65_000.0))
    }

    @Test
    fun `effective sats edge cases are zero`() {
        assertEquals(0L, effective("100.00", SendAmountUnit.USD, 0.0))
        assertEquals(0L, effective("100.00", SendAmountUnit.USD, -50_000.0))
        assertEquals(0L, effective("", SendAmountUnit.SATS, 65_000.0))
        assertEquals(0L, effective("   ", SendAmountUnit.USD, 65_000.0))
        assertEquals(0L, effective("abc", SendAmountUnit.BTC, 65_000.0))
        assertEquals(0L, effective("-50", SendAmountUnit.SATS, 65_000.0))
        assertEquals(0L, effective("-10.00", SendAmountUnit.USD, 65_000.0))
        assertEquals(0L, effective("99999999999999999999999999", SendAmountUnit.SATS, 65_000.0))
    }

    @Test
    fun `floored division prevents sub-satoshi rounding overdraw`() {
        // $0.01 at $60,000/BTC = 16.666... sats, must floor to 16, never 17
        assertEquals(16L, effective("0.01", SendAmountUnit.USD, 60_000.0))
    }

    @Test
    fun `switch unit preserves value`() {
        val price = 65_000.0
        assertEquals(
            "100000",
            SendAmountCalculator.switchUnit(
                SendAmountUnit.USD,
                SendAmountUnit.SATS,
                "65.00",
                price,
            ),
        )
        assertEquals(
            "0.00100000",
            SendAmountCalculator.switchUnit(
                SendAmountUnit.SATS,
                SendAmountUnit.BTC,
                "100000",
                price,
            ),
        )
        assertEquals(
            "65.00",
            SendAmountCalculator.switchUnit(
                SendAmountUnit.BTC,
                SendAmountUnit.USD,
                "0.00100000",
                price,
            ),
        )
        assertEquals(
            "",
            SendAmountCalculator.switchUnit(SendAmountUnit.USD, SendAmountUnit.SATS, "", price),
        )
        assertEquals(
            "",
            SendAmountCalculator.switchUnit(SendAmountUnit.SATS, SendAmountUnit.USD, "10000", 0.0),
        )
    }

    @Test
    fun `percentage of balance in each unit`() {
        val price = 65_000.0
        val total = 200_000L
        assertEquals(
            "100000",
            SendAmountCalculator.percentageOf(50, total, SendAmountUnit.SATS, price),
        )
        assertEquals(
            "65.00",
            SendAmountCalculator.percentageOf(50, total, SendAmountUnit.USD, price),
        )
        assertEquals(
            "0.00100000",
            SendAmountCalculator.percentageOf(50, total, SendAmountUnit.BTC, price),
        )
        assertEquals("", SendAmountCalculator.percentageOf(50, 0L, SendAmountUnit.USD, price))
        assertEquals("", SendAmountCalculator.percentageOf(50, total, SendAmountUnit.USD, 0.0))
    }

    @Test
    fun `normalize input pads and cleans`() {
        assertEquals("12.00", SendAmountCalculator.normalizeInput("12", SendAmountUnit.USD))
        assertEquals("12.50", SendAmountCalculator.normalizeInput("12.5", SendAmountUnit.USD))
        assertEquals("12.00", SendAmountCalculator.normalizeInput("12.", SendAmountUnit.USD))
        assertEquals("0.50", SendAmountCalculator.normalizeInput(".5", SendAmountUnit.USD))
        assertEquals("50", SendAmountCalculator.normalizeInput("0050", SendAmountUnit.SATS))
        assertEquals("0.001", SendAmountCalculator.normalizeInput(".001", SendAmountUnit.BTC))
        assertEquals("", SendAmountCalculator.normalizeInput("", SendAmountUnit.USD))
    }

    @Test
    fun `normalize input truncates and never rounds up`() {
        assertEquals("100", SendAmountCalculator.normalizeInput("100.5", SendAmountUnit.SATS))
        assertEquals(
            "0.12345678",
            SendAmountCalculator.normalizeInput("0.123456789", SendAmountUnit.BTC),
        )
        assertEquals("12.34", SendAmountCalculator.normalizeInput("12.345", SendAmountUnit.USD))
        assertEquals("12.34", SendAmountCalculator.normalizeInput("12.349", SendAmountUnit.USD))
    }

    @Test
    fun `unit max decimals`() {
        assertEquals(2, SendAmountUnit.USD.maxDecimals)
        assertEquals(0, SendAmountUnit.SATS.maxDecimals)
        assertEquals(8, SendAmountUnit.BTC.maxDecimals)
    }

    @Test
    fun `effective sats reads onchain destination amount when input is empty`() {
        val dest = SendDestination.Onchain("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", 2500L)
        assertEquals(
            2500L,
            SendAmountCalculator.effectiveSats(dest, "", SendAmountUnit.SATS, 50_000.0),
        )
        assertEquals(
            2500L,
            SendAmountCalculator.effectiveSats(dest, "   ", SendAmountUnit.USD, 50_000.0),
        )
        // A typed amount takes precedence over the URI amount.
        assertEquals(
            5000L,
            SendAmountCalculator.effectiveSats(dest, "5000", SendAmountUnit.SATS, 50_000.0),
        )
    }

    @Test
    fun `effective sats uses fixed bolt11 amount over typed text`() {
        val dest = SendDestination.Bolt11("lnbc1", 2_500_999L)
        assertEquals(
            2500L,
            SendAmountCalculator.effectiveSats(dest, "9999", SendAmountUnit.SATS, 50_000.0),
        )
    }

    @Test
    fun `effective sats is exact for inputs that are inexact as doubles`() {
        val price = 65_000.0
        assertEquals(30_000L, effective("0.0003", SendAmountUnit.BTC, price))
        assertEquals(60_000L, effective("0.0006", SendAmountUnit.BTC, price))
        assertEquals(120_000L, effective("0.0012", SendAmountUnit.BTC, price))
        assertEquals(1L, effective("0.00000001", SendAmountUnit.BTC, price))
        assertEquals(7_000L, effective("4.55", SendAmountUnit.USD, price))
        // Sub-sat precision is floored, never rounded up.
        assertEquals(1L, effective("0.000000019", SendAmountUnit.BTC, price))
        assertEquals(25L, effective("0.015", SendAmountUnit.USD, 60_000.0))
    }

    @Test
    fun `direct conversions are exact`() {
        assertEquals(30_000L, SendAmountCalculator.satsFromBtc(BigDecimal("0.0003")))
        assertEquals(7_000L, SendAmountCalculator.satsFromUsd(BigDecimal("4.55"), 65_000.0))
        assertNull(SendAmountCalculator.satsFromUsd(BigDecimal("4.55"), Double.NaN))
        assertNull(SendAmountCalculator.satsFromBtc(BigDecimal("0.000000009")))
        assertNull(SendAmountCalculator.satsFromBtc(BigDecimal("21000000.00000001")))
        assertEquals(650L, SendAmountCalculator.centsFromSats(10_001L, 65_000.0))
        assertNull(SendAmountCalculator.centsFromSats(-1L, 65_000.0))
        assertEquals("0.0003", SendAmountCalculator.btcString(30_000L, trimTrailingZeros = true))
        assertEquals("1", SendAmountCalculator.btcString(100_000_000L, trimTrailingZeros = true))
        assertEquals("0.05", SendAmountCalculator.usdString(5L))
    }

    @Test
    fun `parse decimal accepts leading and trailing point`() {
        assertEquals(BigDecimal("0.5"), SendAmountCalculator.parseDecimal(".5"))
        assertEquals(BigDecimal("5"), SendAmountCalculator.parseDecimal("5."))
    }

    @Test
    fun `effective sats rejects malformed numbers`() {
        assertEquals(0L, effective("1e5", SendAmountUnit.SATS, 65_000.0))
        assertEquals(0L, effective("1.2.3", SendAmountUnit.BTC, 65_000.0))
        assertEquals(0L, effective("+5", SendAmountUnit.SATS, 65_000.0))
        assertEquals(0L, effective("0x10", SendAmountUnit.SATS, 65_000.0))
    }

    @Test
    fun `format sats as btc is exact and round trips without drift`() {
        val price = 65_000.0
        assertEquals(
            "0.00030000",
            SendAmountCalculator.formatSats(30_000L, SendAmountUnit.BTC, price),
        )
        assertEquals(
            "0.12345678",
            SendAmountCalculator.formatSats(12_345_678L, SendAmountUnit.BTC, price),
        )
        assertEquals("0.00000001", SendAmountCalculator.formatSats(1L, SendAmountUnit.BTC, price))

        for (sats in listOf(1L, 30_000L, 59_999L, 119_999L, 12_345_678L, 2_100_000_000_000_000L)) {
            val btcText = SendAmountCalculator.formatSats(sats, SendAmountUnit.BTC, price)
            assertEquals(
                "round-trip drifted for $sats sats via $btcText",
                sats.toString(),
                SendAmountCalculator.switchUnit(
                    SendAmountUnit.BTC,
                    SendAmountUnit.SATS,
                    btcText,
                    price,
                ),
            )
        }
    }

    @Test
    fun `format sats as usd truncates to cents`() {
        // 10,001 sats at $65,000 = $6.50065 -> $6.50, never $6.51.
        assertEquals("6.50", SendAmountCalculator.formatSats(10_001L, SendAmountUnit.USD, 65_000.0))
        // 1 sat at $65,000 is below a cent.
        assertEquals("0.00", SendAmountCalculator.formatSats(1L, SendAmountUnit.USD, 65_000.0))
    }
}
