package com.stablechannels.app.ui.home.balancebar

import org.junit.Assert.*
import org.junit.Test

class BalanceBarTradeCalculatorTest {

    @Test
    fun emptyBalanceReturnsInvalidTrade() {
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.5f,
                targetFraction = 0.7f,
                totalUSD = 0.0,
                stableUSD = 0.0,
                maxSellUSD = 0.0,
            )

        assertFalse(evaluation.isValidTrade)
        assertNull(evaluation.direction)
        assertEquals(0.0, evaluation.clampedUSD, 0.001)
    }

    @Test
    fun buyUnderMinimumOneDollarIsRejected() {
        // totalUSD = 100, drag left from 0.50 to 0.495 (delta 0.005 -> $0.50)
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.50f,
                targetFraction = 0.495f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 50.0,
            )

        assertEquals(TradeDirection.BUY, evaluation.direction)
        assertEquals(0.5, evaluation.requestedUSD, 0.001)
        assertFalse(evaluation.isValidTrade)
    }

    @Test
    fun buyAtOrAboveOneDollarIsAccepted() {
        // totalUSD = 100, drag left from 0.50 to 0.49 (delta 0.01 -> $1.00)
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.50f,
                targetFraction = 0.49f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 50.0,
            )

        assertEquals(TradeDirection.BUY, evaluation.direction)
        assertEquals(1.0, evaluation.clampedUSD, 0.001)
        assertTrue(evaluation.isValidTrade)
    }

    @Test
    fun buyBeyondStableBalanceClampsToAvailableStableUSD() {
        // totalUSD = 100, stableUSD = 5.0, drag from 0.10 to 0.0 (delta 0.10 -> $10 requested)
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.10f,
                targetFraction = 0.0f,
                totalUSD = 100.0,
                stableUSD = 5.0,
                maxSellUSD = 50.0,
            )

        assertEquals(TradeDirection.BUY, evaluation.direction)
        assertEquals(10.0, evaluation.requestedUSD, 0.001)
        assertEquals(5.0, evaluation.clampedUSD, 0.001)
        assertTrue(evaluation.isValidTrade)
    }

    @Test
    fun sellBeyondMaxSellUSDClampsToLimit() {
        // totalUSD = 100, maxSellUSD = 20.0, drag from 0.50 to 0.80 ($30 requested)
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.50f,
                targetFraction = 0.80f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 20.0,
            )

        assertEquals(TradeDirection.SELL, evaluation.direction)
        assertEquals(30.0, evaluation.requestedUSD, 0.001)
        assertEquals(20.0, evaluation.clampedUSD, 0.001)
        assertTrue(evaluation.isValidTrade)
    }

    @Test
    fun sellExactlyMaxSellUSDIsAccepted() {
        // totalUSD = 100, maxSellUSD = 25.0, drag from 0.50 to 0.75 ($25 requested)
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.50f,
                targetFraction = 0.75f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 25.0,
            )

        assertEquals(TradeDirection.SELL, evaluation.direction)
        assertEquals(25.0, evaluation.clampedUSD, 0.001)
        assertTrue(evaluation.isValidTrade)
    }

    @Test
    fun clampFractionBeyondPhysicalBounds() {
        // Raw fraction < 0
        val under =
            BalanceBarTradeCalculator.clampFraction(
                initialFraction = 0.5f,
                rawFraction = -0.2f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 50.0,
            )
        assertEquals(0.0f, under.fraction, 0.001f)
        assertFalse(under.isAtSellLimit)

        // Raw fraction > 1
        val over =
            BalanceBarTradeCalculator.clampFraction(
                initialFraction = 0.5f,
                rawFraction = 1.5f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 50.0,
            )
        assertEquals(1.0f, over.fraction, 0.001f)
    }

    @Test
    fun clampFractionEnforcesSellLimitFlag() {
        // totalUSD = 100, maxSellUSD = 20. Max allowed fraction = 0.5 + 0.2 = 0.7
        val atLimit =
            BalanceBarTradeCalculator.clampFraction(
                initialFraction = 0.5f,
                rawFraction = 0.75f,
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 20.0,
            )

        assertEquals(0.7f, atLimit.fraction, 0.001f)
        assertTrue(atLimit.isAtSellLimit)
    }

    @Test
    fun zeroBtcPriceTradeClampingAndSelection() {
        // btcPrice = 0 -> nativeUSD = 0, totalUSD = stableUSD = 50.0
        val totalUSD = 50.0
        val stableUSD = 50.0
        val maxSellUSD = 0.0

        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 1.0f,
                targetFraction = 0.8f,
                totalUSD = totalUSD,
                stableUSD = stableUSD,
                maxSellUSD = maxSellUSD,
            )

        assertEquals(TradeDirection.BUY, evaluation.direction)
        assertEquals(10.0, evaluation.clampedUSD, 0.001)
        assertTrue(evaluation.isValidTrade)
    }

    @Test
    fun btcOnlyBalanceCannotBuyStableUSD() {
        // stableUSD = 0, totalUSD = 100.0
        val tryBuy =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = 0.0f,
                targetFraction = -0.1f,
                totalUSD = 100.0,
                stableUSD = 0.0,
                maxSellUSD = 50.0,
            )

        assertEquals(0.0, tryBuy.clampedUSD, 0.001)
        assertFalse(tryBuy.isValidTrade)
    }

    @Test
    fun interactionMathTranslationAndTapDetection() {
        val target =
            BalanceBarTradeCalculator.calculateTargetFraction(
                initialFraction = 0.4f,
                translationX = 75.0f,
                barWidth = 300.0f,
            )
        // 0.4 + 75/300 = 0.4 + 0.25 = 0.65
        assertEquals(0.65f, target, 0.001f)

        // Thumb position alignment
        val thumbX =
            BalanceBarTradeCalculator.calculateThumbPosition(
                fraction = 0.65f,
                barWidth = 300.0f,
            )
        assertEquals(195.0f, thumbX, 0.001f)

        assertTrue(
            BalanceBarTradeCalculator.isTap(
                translationX = 3.0f,
                translationY = 2.0f,
                threshold = 5.0f,
            )
        )
        assertFalse(
            BalanceBarTradeCalculator.isTap(
                translationX = 6.0f,
                translationY = 0.0f,
                threshold = 5.0f,
            )
        )
    }
}
