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

    @Test
    fun fundsArrivingMidDragFromEmptyStateDoesNotExecuteTrade() {
        var tradeRequested: TradeRequest? = null
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
            onTradeRequest = { tradeRequested = it },
            onEmptyInteraction = null,
        )

        state.onDragStart(Offset(150f, 10f))
        assertTrue(state.isDragging)
        assertTrue(state.startedEmpty)

        // Drag 50px
        state.onDrag(dragAmountX = 50f)
        assertEquals(50f, state.dragOffsetPx, 0.001f)

        // Mid-drag, incoming funds arrive and channel is no longer empty
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = { tradeRequested = it },
            onEmptyInteraction = null,
        )

        // Release drag
        state.onDragEnd()

        // MUST NOT execute a trade, must trigger snap-back and show deposit prompt
        org.junit.Assert.assertNull(tradeRequested)
        assertFalse(state.isDragging)
        assertFalse(state.startedEmpty)
        assertTrue(state.showDepositPrompt)
        assertEquals(1, snapBack.animateCallCount)
    }

    @Test
    fun dragStartBaseFractionCapturedAndPreservedDuringPriceTicks() {
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
            stableUSD = 40.0,
            maxSellUSD = 40.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        org.junit.Assert.assertNull(state.dragStartBaseFraction)

        // Start drag at thumb center (40% mark: 20px + 0.4 * 260px = 124px)
        state.onDragStart(Offset(124f, 10f))
        assertTrue(state.isDragging)
        assertEquals(0.4f, state.dragStartBaseFraction ?: 0f, 0.001f)

        // Price changes mid-drag changing canonical ratio to 40 / 200 = 0.2
        state.updateInputs(
            totalUSD = 200.0,
            stableUSD = 40.0,
            maxSellUSD = 40.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        // Base fraction MUST remain latched to initial 0.4f during the drag
        assertEquals(0.4f, state.dragStartBaseFraction ?: 0f, 0.001f)

        state.onDragCancel()
        assertFalse(state.isDragging)
        org.junit.Assert.assertNull(state.dragStartBaseFraction)
    }

    @Test
    fun sellLimitWarningHapticIsDebounced() {
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
            stableUSD = 20.0,
            maxSellUSD = 25.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        // Base is 20/100 = 0.2, usable width = 260. baseXPx = 20 + 0.2*260 = 72px
        state.onDragStart(Offset(72f, 10f))

        // Drag right past sell limit (+25 USD = delta fraction 0.25 -> 65px)
        state.onDrag(dragAmountX = 80f)
        assertTrue(state.atSellLimit)
        assertEquals(1, spy.warningCount)

        // Drag slightly left below limit, then immediately back past limit within debounce window
        state.onDrag(dragAmountX = -30f)
        assertFalse(state.atSellLimit)

        state.onDrag(dragAmountX = 30f)
        assertTrue(state.atSellLimit)
        // Warning count MUST still be 1 due to 500ms debounce
        assertEquals(1, spy.warningCount)
    }

    @Test
    fun tradeRequestRetainsBaseFractionUntilSnapBackCompletes() {
        var tradeRequested: TradeRequest? = null
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
            onTradeRequest = { tradeRequested = it },
            onEmptyInteraction = null,
        )

        // Grab thumb at 50%
        state.onDragStart(Offset(150f, 10f))
        state.onDrag(dragAmountX = 40f)

        state.onDragEnd()
        org.junit.Assert.assertNotNull(tradeRequested)

        // Base fraction MUST remain latched while the trade modal is open
        assertEquals(0.5f, state.dragStartBaseFraction ?: 0f, 0.001f)

        // When trading concludes, snap-back clears it
        state.triggerSnapBack(state.dragOffsetPx)
        org.junit.Assert.assertNull(state.dragStartBaseFraction)
    }

    @Test
    fun snapBackAdjustsOffsetByBaseDifferenceOnTradeCompletion() {
        var tradeRequested: TradeRequest? = null
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
            onTradeRequest = { tradeRequested = it },
            onEmptyInteraction = null,
        )

        // Base is 50/100 = 0.5. usable width = 300 - 40 = 260px. baseXPx = 20 + 0.5*260 = 150px
        state.onDragStart(Offset(150f, 10f))
        state.onDrag(dragAmountX = 40f)
        assertEquals(40f, state.dragOffsetPx, 0.001f)

        state.onDragEnd()
        org.junit.Assert.assertNotNull(tradeRequested)

        // Latched at 0.5f while trading
        assertEquals(0.5f, state.dragStartBaseFraction ?: 0f, 0.001f)

        // Trade completes and balance updates to 30 USD stable (canonical fraction = 30/100 = 0.3f)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 30.0,
            maxSellUSD = 30.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        // Test rebase = false (e.g. Awakening takes over thumb positioning)
        state.triggerSnapBack(state.dragOffsetPx, rebase = false)
        org.junit.Assert.assertNull(state.dragStartBaseFraction)
        assertEquals(40f, snapBack.lastFromOffset ?: 0f, 0.001f)

        // Reset and test rebase = true (e.g. Reduce Motion or trade modal completion)
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = { tradeRequested = it },
            onEmptyInteraction = null,
        )
        state.onDragStart(Offset(150f, 10f))
        state.onDrag(dragAmountX = 40f)
        state.onDragEnd()
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 30.0,
            maxSellUSD = 30.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = null,
            onEmptyInteraction = null,
        )

        state.triggerSnapBack(state.dragOffsetPx, rebase = true)
        org.junit.Assert.assertNull(state.dragStartBaseFraction)
        // lastFromOffset == 40 + (0.5 - 0.3) * 260 = 40 + 52 = 92
        assertEquals(92f, snapBack.lastFromOffset ?: 0f, 0.001f)
    }

    @Test
    fun onDragCancelResetsDraggingAndTriggersSnapBack() {
        var tradeRequested: TradeRequest? = null
        val snapBack = FakeBalanceBarSnapBack()
        val spy = SpyBalanceBarHaptics()
        val state =
            BalanceBarState(
                scope = testScope,
                snapBack = snapBack,
                haptics = spy,
            )
        state.updateInputs(
            totalUSD = 100.0,
            stableUSD = 50.0,
            maxSellUSD = 50.0,
            isEmpty = false,
            density = 2f,
            onDragStarted = null,
            onTradeRequest = { tradeRequested = it },
            onEmptyInteraction = null,
        )
        state.updateLayout(barWidthPx = 300f, thumbDiameterPx = 40f)

        state.onDragStart(Offset(150f, 10f))
        state.onDrag(dragAmountX = 50f)
        assertTrue(state.isDragging)
        assertEquals(50f, state.dragOffsetPx, 0.001f)

        state.onDragCancel(rebase = true)

        assertFalse(state.isDragging)
        org.junit.Assert.assertNull(tradeRequested)
        assertEquals(50f, snapBack.lastFromOffset ?: 0f, 0.001f)
    }
}
