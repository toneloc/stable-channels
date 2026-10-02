package com.stablechannels.app.ui.home.balancebar

import android.view.View
import androidx.compose.animation.core.Animatable
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerInputChange
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mockito.Mockito.mock

@OptIn(ExperimentalCoroutinesApi::class)
class BalanceBarStateTest {

    private val testDispatcher = UnconfinedTestDispatcher()
    private val testScope = TestScope(testDispatcher)
    private val mockView: View = mock(View::class.java)

    private fun createMockChange(): PointerInputChange {
        return mock(PointerInputChange::class.java)
    }

    @Test
    fun emptyStateSmallDragActsAsTapAndTriggersEmptyInteraction() {
        var emptyInteractionCalled = false
        val snapBackAnim = Animatable(0f)
        val state =
            BalanceBarState(
                totalUSD = 0.0,
                stableUSD = 0.0,
                maxSellUSD = 0.0,
                isEmpty = true,
                density = 2f,
                view = mockView,
                scope = testScope,
                snapBackAnim = snapBackAnim,
                onDragStarted = null,
                onTradeRequest = null,
                onEmptyInteraction = { emptyInteractionCalled = true },
            )

        // Base 50% on 300px bar is 150px
        state.onDragStart(Offset(150f, 10f), baseXPx = 150f, thumbDiameterPx = 40f)
        assertTrue(state.isDragging)

        // Move by only 4px (threshold is 5 * 2 = 10px)
        state.onDrag(
            change = createMockChange(),
            dragAmount = Offset(4f, 0f),
            baseXPx = 150f,
            barWidthPx = 300f,
            maxSellOffset = 0f,
        )

        state.onDragEnd(barWidthPx = 300f)

        assertFalse(state.isDragging)
        assertTrue(emptyInteractionCalled)
        assertEquals(0f, state.dragOffsetPx, 0.001f)
    }

    @Test
    fun emptyStatePlaygroundDragEnablesFreeMovementAndElasticSnapBack() {
        var emptyInteractionCalled = false
        val snapBackAnim = Animatable(0f)
        val state =
            BalanceBarState(
                totalUSD = 0.0,
                stableUSD = 0.0,
                maxSellUSD = 0.0,
                isEmpty = true,
                density = 2f,
                view = mockView,
                scope = testScope,
                snapBackAnim = snapBackAnim,
                onDragStarted = null,
                onTradeRequest = null,
                onEmptyInteraction = { emptyInteractionCalled = true },
            )

        state.onDragStart(Offset(150f, 10f), baseXPx = 150f, thumbDiameterPx = 40f)
        assertTrue(state.isDragging)

        // Drag 60px to the right
        state.onDrag(
            change = createMockChange(),
            dragAmount = Offset(60f, 0f),
            baseXPx = 150f,
            barWidthPx = 300f,
            maxSellOffset = 0f,
        )
        assertEquals(60f, state.dragOffsetPx, 0.001f)

        // Drop
        state.onDragEnd(barWidthPx = 300f)

        assertFalse(state.isDragging)
        // Drag and drop must NOT immediately open receive screen
        assertFalse(emptyInteractionCalled)
        // Must show deposit prompt affordance
        assertTrue(state.showDepositPrompt)
    }

    @Test
    fun nonEmptyStateClampsToMaxSellLimit() {
        val snapBackAnim = Animatable(0f)
        val state =
            BalanceBarState(
                totalUSD = 100.0,
                stableUSD = 50.0,
                maxSellUSD = 20.0,
                isEmpty = false,
                density = 2f,
                view = mockView,
                scope = testScope,
                snapBackAnim = snapBackAnim,
                onDragStarted = null,
                onTradeRequest = null,
                onEmptyInteraction = null,
            )

        // barWidth = 300px, baseXPx = 150px (50%)
        // maxSellOffset = 300 * 20 / 100 = 60px
        val maxSellOffset = 60f
        state.onDragStart(Offset(150f, 10f), baseXPx = 150f, thumbDiameterPx = 40f)

        // Drag 80px (beyond 60px limit)
        state.onDrag(
            change = createMockChange(),
            dragAmount = Offset(80f, 0f),
            baseXPx = 150f,
            barWidthPx = 300f,
            maxSellOffset = maxSellOffset,
        )

        assertTrue(state.atSellLimit)
        assertEquals(60f, state.dragOffsetPx, 0.001f)
    }
}
