package com.stablechannels.app.ui.home.balancebar

import android.view.HapticFeedbackConstants
import android.view.View
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.AnimationVector1D
import androidx.compose.animation.core.spring
import androidx.compose.runtime.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerInputChange
import kotlin.math.abs
import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun rememberBalanceBarState(
    totalUSD: Double,
    stableUSD: Double,
    maxSellUSD: Double,
    isEmpty: Boolean,
    density: Float,
    view: View,
    scope: CoroutineScope = rememberCoroutineScope(),
    onDragStarted: (() -> Unit)? = null,
    onTradeRequest: ((TradeDirection, Double) -> Unit)? = null,
    onEmptyInteraction: (() -> Unit)? = null,
): BalanceBarState {
    val snapBackAnim = remember { Animatable(0f) }
    return remember(isEmpty, totalUSD, maxSellUSD, stableUSD) {
        BalanceBarState(
            totalUSD = totalUSD,
            stableUSD = stableUSD,
            maxSellUSD = maxSellUSD,
            isEmpty = isEmpty,
            density = density,
            view = view,
            scope = scope,
            snapBackAnim = snapBackAnim,
            onDragStarted = onDragStarted,
            onTradeRequest = onTradeRequest,
            onEmptyInteraction = onEmptyInteraction,
        )
    }
}

@Stable
class BalanceBarState(
    private val totalUSD: Double,
    private val stableUSD: Double,
    private val maxSellUSD: Double,
    val isEmpty: Boolean,
    private val density: Float,
    private val view: View,
    private val scope: CoroutineScope,
    private val snapBackAnim: Animatable<Float, AnimationVector1D>,
    private val onDragStarted: (() -> Unit)?,
    private val onTradeRequest: ((TradeDirection, Double) -> Unit)?,
    private val onEmptyInteraction: (() -> Unit)?,
) {
    var isDragging by mutableStateOf(false)
        private set

    var isSnappingBack by mutableStateOf(false)
        private set

    var dragOffsetPx by mutableFloatStateOf(0f)
        private set

    var atSellLimit by mutableStateOf(false)
        private set

    var showDepositPrompt by mutableStateOf(false)
        private set

    private var hasTriggeredHaptic = false
    private var totalDragDistance = 0f
    private var depositPromptJob: Job? = null
    private val minTradeUSD = 1.0

    val currentOffsetPx: Float
        get() = if (isSnappingBack) snapBackAnim.value else dragOffsetPx

    fun triggerSnapBack(fromOffset: Float, onFinished: (() -> Unit)? = null) {
        isSnappingBack = true
        scope.launch {
            snapBackAnim.snapTo(fromOffset)
            snapBackAnim.animateTo(
                targetValue = 0f,
                animationSpec = spring(dampingRatio = 0.68f, stiffness = 400f),
            )
            dragOffsetPx = 0f
            isSnappingBack = false
            onFinished?.invoke()
        }
    }

    fun onDragStart(offset: Offset, baseXPx: Float, thumbDiameterPx: Float) {
        val withinThumb = abs(offset.x - baseXPx) < thumbDiameterPx * 1.5f
        if (isEmpty || withinThumb) {
            isDragging = true
            isSnappingBack = false
            hasTriggeredHaptic = false
            atSellLimit = false
            totalDragDistance = 0f
            depositPromptJob?.cancel()
            showDepositPrompt = false
            dragOffsetPx = 0f
            onDragStarted?.invoke()
            view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
        }
    }

    fun onDrag(
        change: PointerInputChange,
        dragAmount: Offset,
        baseXPx: Float,
        barWidthPx: Float,
        maxSellOffset: Float,
    ) {
        if (!isDragging) return
        change.consume()
        totalDragDistance += abs(dragAmount.x)
        val proposedOffset = dragOffsetPx + dragAmount.x

        if (isEmpty) {
            val minOffset = -baseXPx
            val maxOffset = barWidthPx - baseXPx
            dragOffsetPx = proposedOffset.coerceIn(minOffset, maxOffset)
            return
        }

        atSellLimit = proposedOffset > maxSellOffset
        val newOffset = proposedOffset.coerceIn(-baseXPx, maxSellOffset)
        dragOffsetPx = newOffset

        if (!hasTriggeredHaptic && barWidthPx > 0) {
            val fraction = abs(newOffset) / barWidthPx
            val tradeUSD = fraction * totalUSD
            if (tradeUSD >= minTradeUSD) {
                hasTriggeredHaptic = true
                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
            }
        }
    }

    fun onDragEnd(barWidthPx: Float) {
        if (!isDragging) {
            dragOffsetPx = 0f
            return
        }
        isDragging = false

        if (isEmpty) {
            if (totalDragDistance < 5f * density) {
                dragOffsetPx = 0f
                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                onEmptyInteraction?.invoke()
            } else {
                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                showDepositPrompt = true
                depositPromptJob?.cancel()
                depositPromptJob = scope.launch {
                    delay(2500)
                    showDepositPrompt = false
                }
                triggerSnapBack(dragOffsetPx)
            }
            return
        }

        val offset = dragOffsetPx
        val fraction = if (barWidthPx > 0) offset / barWidthPx else 0f
        val tradeUSD = abs(fraction) * totalUSD
        if (tradeUSD < minTradeUSD) {
            triggerSnapBack(offset)
            return
        }
        val direction = if (offset > 0) TradeDirection.SELL else TradeDirection.BUY
        val clamped =
            if (direction == TradeDirection.BUY) min(tradeUSD, stableUSD)
            else min(tradeUSD, maxSellUSD)
        onTradeRequest?.invoke(direction, clamped)
    }

    fun onDragCancel() {
        isDragging = false
        triggerSnapBack(dragOffsetPx)
    }
}
