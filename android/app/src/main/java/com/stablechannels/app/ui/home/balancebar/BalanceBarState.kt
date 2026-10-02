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

/**
 * UI presentation state for the balance bar component. Coordinates UI flags, delegates interaction
 * geometry to BalanceBarInteraction, and financial rules to BalanceBarTradeCalculator.
 */
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
    private var accumulatedTranslationX = 0f
    private var depositPromptJob: Job? = null

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
            accumulatedTranslationX = 0f
            isSnappingBack = false
            onFinished?.invoke()
        }
    }

    fun onDragStart(offset: Offset, baseXPx: Float, thumbDiameterPx: Float) {
        val withinThumb =
            BalanceBarInteraction.isWithinThumb(
                touchX = offset.x,
                thumbX = baseXPx,
                thumbDiameter = thumbDiameterPx,
            )
        if (isEmpty || withinThumb) {
            isDragging = true
            isSnappingBack = false
            hasTriggeredHaptic = false
            atSellLimit = false
            totalDragDistance = 0f
            accumulatedTranslationX = 0f
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
    ) {
        if (!isDragging || barWidthPx <= 0f) return
        change.consume()
        totalDragDistance += abs(dragAmount.x)
        accumulatedTranslationX += dragAmount.x

        val baseFraction = if (isEmpty) 0.5f else baseXPx / barWidthPx
        val rawFraction =
            BalanceBarInteraction.calculateTargetFraction(
                initialFraction = baseFraction,
                translationX = accumulatedTranslationX,
                barWidth = barWidthPx,
            )

        if (isEmpty) {
            dragOffsetPx = (rawFraction - 0.5f) * barWidthPx
            return
        }

        val clampedResult =
            BalanceBarTradeCalculator.clampFraction(
                initialFraction = baseFraction,
                rawFraction = rawFraction,
                totalUSD = totalUSD,
                stableUSD = stableUSD,
                maxSellUSD = maxSellUSD,
            )

        dragOffsetPx = (clampedResult.fraction - baseFraction) * barWidthPx
        atSellLimit = clampedResult.isAtSellLimit

        if (!hasTriggeredHaptic) {
            val evaluation =
                BalanceBarTradeCalculator.calculateSelection(
                    initialFraction = baseFraction,
                    targetFraction = clampedResult.fraction,
                    totalUSD = totalUSD,
                    stableUSD = stableUSD,
                    maxSellUSD = maxSellUSD,
                )
            if (evaluation.isValidTrade) {
                hasTriggeredHaptic = true
                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
            }
        }
    }

    fun onDragEnd(barWidthPx: Float) {
        if (!isDragging) {
            dragOffsetPx = 0f
            accumulatedTranslationX = 0f
            return
        }
        isDragging = false

        if (isEmpty) {
            if (BalanceBarInteraction.isTap(totalDragDistance, threshold = 5f * density)) {
                dragOffsetPx = 0f
                accumulatedTranslationX = 0f
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

        val baseFraction = if (barWidthPx > 0) (baseXPx(barWidthPx) / barWidthPx) else 0.5f
        val targetFraction =
            if (barWidthPx > 0) baseFraction + (dragOffsetPx / barWidthPx) else baseFraction
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = baseFraction,
                targetFraction = targetFraction,
                totalUSD = totalUSD,
                stableUSD = stableUSD,
                maxSellUSD = maxSellUSD,
            )

        if (evaluation.isValidTrade && evaluation.direction != null) {
            onTradeRequest?.invoke(evaluation.direction, evaluation.clampedUSD)
        } else {
            triggerSnapBack(dragOffsetPx)
        }
    }

    fun onDragCancel() {
        isDragging = false
        triggerSnapBack(dragOffsetPx)
    }

    private fun baseXPx(barWidthPx: Float): Float {
        return if (totalUSD > 0.0) {
            (barWidthPx * (stableUSD / totalUSD).coerceIn(0.0, 1.0)).toFloat()
        } else {
            barWidthPx * 0.5f
        }
    }
}
