package com.stablechannels.app.ui.home.balancebar

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

private class SpyBalanceBarHaptics : BalanceBarHaptics {
    var tickCount = 0
    var impactCount = 0
    var warningCount = 0

    override fun tick() {
        tickCount++
    }

    override fun impact() {
        impactCount++
    }

    override fun warning() {
        warningCount++
    }
}

private class FakeBalanceBarSnapBack(var currentValue: Float = 0f) : BalanceBarSnapBack {
    var animateCallCount = 0
    var lastFromOffset: Float? = null

    override val value: Float
        get() = currentValue

    override suspend fun animateToZero(fromOffset: Float) {
        animateCallCount++
        lastFromOffset = fromOffset
        currentValue = 0f
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class BalanceBarStateTest {

    private val testDispatcher = UnconfinedTestDispatcher()
    private val testScope = TestScope(testDispatcher)

    @Test
    fun emptyStateTapTriggersEmptyInteraction() {
        var emptyInteractionCalled = false
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 0.0,
            stableUSD = 0.0,
            maxSellUSD = 0.0,
            isEmpty = true,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = { emptyInteractionCalled = true },
        )

        state.onTap(Offset(150f, 10f))

        assertTrue(emptyInteractionCalled)
        assertEquals(1, spy.tickCount)
    }

    @Test
    fun emptyStatePlaygroundDragEnablesFreeMovementAndTriggersSnapBack() {
        var emptyInteractionCalled = false
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 0.0,
            stableUSD = 0.0,
            maxSellUSD = 0.0,
            isEmpty = true,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = { emptyInteractionCalled = true },
        )

        state.onDragStart(Offset(150f, 10f))
        assertTrue(state.isDragging)

        // Drag 60px to the right
        state.onDrag(dragAmountX = 60f)
        assertEquals(60f, state.dragOffsetPx, 0.001f)

        // Drop
        state.onDragEnd()

        assertFalse(state.isDragging)
        assertFalse(emptyInteractionCalled)
        assertTrue(state.showDepositPrompt)
        assertEquals(1, snapBack.animateCallCount)
        assertEquals(60f, snapBack.lastFromOffset ?: 0f, 0.001f)
    }

    @Test
    fun priceTickMidGesturePreservesDragStateWithoutResetting() {
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        state.onDragStart(Offset(150f, 10f))
        assertTrue(state.isDragging)

        state.onDrag(dragAmountX = 30f)
        assertEquals(30f, state.dragOffsetPx, 0.001f)

        // Simulate incoming price update changing totalUSD
        state.updateInputs(
            totalUSD = 110.0,
            stableUSD = 50.0,
            maxSellUSD = 55.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        // State MUST remain actively dragging at the same offset
        assertTrue(state.isDragging)
        assertEquals(30f, state.dragOffsetPx, 0.001f)
        assertEquals(110.0, state.totalUSD, 0.001)

        // Verify delivered trade on release matches the dragged delta and direction after price
        // tick
        var deliveredRequest: TradeRequest? = null
        state.updateInputs(
            totalUSD = 110.0,
            stableUSD = 50.0,
            maxSellUSD = 55.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = { deliveredRequest = it },
            onEmptyInteraction = null,
        )

        state.onDragEnd()

        assertNotNull(deliveredRequest)
        assertEquals(TradeDirection.SELL, deliveredRequest?.direction)
        // delta fraction = 30 / 260 = 0.1153846 -> amountUSD = 0.1153846 * 110.0 = 12.69
        assertEquals(12.69, deliveredRequest?.amountUSD ?: 0.0, 0.01)
    }

    @Test
    fun nonEmptyStateClampsToMaxSellLimitAndDebouncesHaptic() {
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 20.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        // barWidth = 300px, thumbDiameter = 40px -> usableWidth = 260px, baseXPx = 150px (50%)
        // maxSellOffset = 260 * 20 / 100 = 52px
        state.onDragStart(Offset(150f, 10f))

        // Drag 80px (beyond 52px limit)
        state.onDrag(dragAmountX = 80f)
        assertTrue(state.atSellLimit)
        assertEquals(52f, state.dragOffsetPx, 0.001f)
        assertEquals(1, spy.warningCount)

        // Subsequent drag while still beyond limit must NOT fire warning again (edge detection)
        state.onDrag(dragAmountX = 10f)
        assertTrue(state.atSellLimit)
        assertEquals(1, spy.warningCount)
    }

    @Test
    fun tradeRequestDeliversFormattedTradeRequestObject() {
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        var deliveredRequest: TradeRequest? = null
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = { deliveredRequest = it },
            onEmptyInteraction = null,
        )

        state.onDragStart(Offset(150f, 10f))
        // Drag 52px right on 260px usable travel -> 52/260 = 0.20 -> $20.0 SELL
        state.onDrag(dragAmountX = 52f)
        state.onDragEnd()

        assertNotNull(deliveredRequest)
        assertEquals(TradeDirection.SELL, deliveredRequest?.direction)
        assertEquals(20.0, deliveredRequest?.amountUSD ?: 0.0, 0.001)
        assertEquals(1, spy.impactCount)
        assertEquals(0, snapBack.animateCallCount)
    }

    @Test
    fun nullOnTradeRequestSnapsBackWithoutLeavingThumbStranded() {
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null, // No trade request handler available (e.g., channel opening)
            onEmptyInteraction = null,
        )

        state.onDragStart(Offset(150f, 10f))
        state.onDrag(dragAmountX = 52f)
        assertEquals(52f, state.dragOffsetPx, 0.001f)

        state.onDragEnd()

        assertFalse(state.isDragging)
        assertEquals(0, spy.impactCount)
        assertEquals(1, snapBack.animateCallCount)
        assertEquals(52f, snapBack.lastFromOffset ?: 0f, 0.001f)
    }

    @Test
    fun snapBackIsCancelledWhenNewDragStartsMidAnimation() {
        val snapBack = FakeBalanceBarSnapBack(currentValue = 30f)
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        state.triggerSnapBack(30f)
        // User immediately grabs thumb at 150px
        state.onDragStart(Offset(150f, 10f))

        assertTrue(state.isDragging)
        assertFalse(state.isSnappingBack)
        assertEquals(0f, state.dragOffsetPx, 0.001f)
    }

    @Test
    fun balanceBarDefaultsAndEmptyPredicateMatchInvariants() {
        assertEquals(22.dp, BalanceBarDefaults.THUMB_DIAMETER)

        // Zero balances are empty
        assertTrue(BalanceBarDefaults.isChannelEmpty(totalSats = 0L, stableUSD = 0.0))
        assertTrue(BalanceBarDefaults.isChannelEmpty(totalSats = -1L, stableUSD = 0.0))

        // Funded stable or satoshi balance is not empty
        assertFalse(BalanceBarDefaults.isChannelEmpty(totalSats = 100_000L, stableUSD = 0.0))
        assertFalse(BalanceBarDefaults.isChannelEmpty(totalSats = 0L, stableUSD = 10.0))
        assertFalse(BalanceBarDefaults.isChannelEmpty(totalSats = 50_000L, stableUSD = 25.0))
    }
}
