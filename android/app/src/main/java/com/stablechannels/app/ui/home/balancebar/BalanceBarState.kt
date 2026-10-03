package com.stablechannels.app.ui.home.balancebar

import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.*
import androidx.compose.ui.geometry.Offset
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
    haptics: BalanceBarHaptics,
    scope: CoroutineScope = rememberCoroutineScope(),
    onDragStarted: (() -> Unit)? = null,
    onTradeRequest: ((TradeRequest) -> Unit)? = null,
    onEmptyInteraction: (() -> Unit)? = null,
): BalanceBarState {
    val snapBackAnim = remember { Animatable(0f) }
    val snapBack = remember(snapBackAnim) { DefaultBalanceBarSnapBack(snapBackAnim) }
    val state = remember { BalanceBarState(scope = scope, snapBack = snapBack, haptics = haptics) }

    val currentOnDragStarted by rememberUpdatedState(onDragStarted)
    val currentOnTradeRequest by rememberUpdatedState(onTradeRequest)
    val currentOnEmptyInteraction by rememberUpdatedState(onEmptyInteraction)

    SideEffect {
        state.updateInputs(
            totalUSD = totalUSD,
            stableUSD = stableUSD,
            maxSellUSD = maxSellUSD,
            isEmpty = isEmpty,
            density = density,
            onDragStarted = currentOnDragStarted,
            onTradeRequest = currentOnTradeRequest,
            onEmptyInteraction = currentOnEmptyInteraction,
        )
    }

    return state
}

/**
 * UI presentation state for the balance bar component. Stable across price updates; coordinates UI
 * flags and delegates geometry and financial rules to BalanceBarTradeCalculator, and haptics to
 * BalanceBarHaptics.
 */
@Stable
class BalanceBarState(
    private val scope: CoroutineScope,
    private val snapBack: BalanceBarSnapBack,
    private val haptics: BalanceBarHaptics,
) {
    var totalUSD by mutableDoubleStateOf(0.0)
        private set

    var stableUSD by mutableDoubleStateOf(0.0)
        private set

    var maxSellUSD by mutableDoubleStateOf(0.0)
        private set

    var isEmpty by mutableStateOf(false)
        private set

    var density by mutableFloatStateOf(1f)
        private set

    var barWidthPx by mutableFloatStateOf(0f)
        private set

    var thumbDiameterPx by mutableFloatStateOf(0f)
        private set

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

    private var onDragStarted: (() -> Unit)? = null
    private var onTradeRequest: ((TradeRequest) -> Unit)? = null
    private var onEmptyInteraction: (() -> Unit)? = null

    private var hasTriggeredHaptic = false
    private var totalDragDistance = 0f
    private var accumulatedTranslationX = 0f
    private var depositPromptJob: Job? = null
    private var snapBackJob: Job? = null

    private val baseFraction: Float
        get() =
            if (isEmpty) 0.5f
            else if (totalUSD > 0.0) (stableUSD / totalUSD).coerceIn(0.0, 1.0).toFloat() else 0.5f

    val baseXPx: Float
        get() {
            if (barWidthPx <= 0f) return 0f
            return BalanceBarTradeCalculator.calculateThumbPosition(
                fraction = baseFraction,
                barWidth = barWidthPx,
                thumbDiameter = thumbDiameterPx,
            )
        }

    val currentOffsetPx: Float
        get() = if (isSnappingBack) snapBack.value else dragOffsetPx

    fun updateLayout(barWidthPx: Float, thumbDiameterPx: Float) {
        this.barWidthPx = barWidthPx
        this.thumbDiameterPx = thumbDiameterPx
    }

    fun updateInputs(
        totalUSD: Double,
        stableUSD: Double,
        maxSellUSD: Double,
        isEmpty: Boolean,
        density: Float,
        onDragStarted: (() -> Unit)?,
        onTradeRequest: ((TradeRequest) -> Unit)?,
        onEmptyInteraction: (() -> Unit)?,
    ) {
        this.totalUSD = totalUSD
        this.stableUSD = stableUSD
        this.maxSellUSD = maxSellUSD
        this.isEmpty = isEmpty
        this.density = density
        this.onDragStarted = onDragStarted
        this.onTradeRequest = onTradeRequest
        this.onEmptyInteraction = onEmptyInteraction
    }

    fun triggerSnapBack(fromOffset: Float, onFinished: (() -> Unit)? = null) {
        snapBackJob?.cancel()
        isSnappingBack = true
        snapBackJob = scope.launch {
            try {
                snapBack.animateToZero(fromOffset)
                dragOffsetPx = 0f
                accumulatedTranslationX = 0f
            } finally {
                isSnappingBack = false
                onFinished?.invoke()
            }
        }
    }

    fun onDragStart(offset: Offset) {
        val withinThumb =
            BalanceBarTradeCalculator.isWithinThumb(
                touchX = offset.x,
                thumbX = baseXPx,
                thumbDiameter = thumbDiameterPx,
            )
        if (isEmpty || withinThumb) {
            snapBackJob?.cancel()
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
            haptics.tick()
        }
    }

    fun onDrag(dragAmountX: Float) {
        if (!isDragging || barWidthPx <= 0f) return
        totalDragDistance += abs(dragAmountX)
        accumulatedTranslationX += dragAmountX

        val rawFraction =
            BalanceBarTradeCalculator.calculateTargetFraction(
                initialFraction = baseFraction,
                translationX = accumulatedTranslationX,
                barWidth = barWidthPx,
                thumbDiameter = thumbDiameterPx,
            )

        val usableWidth = (barWidthPx - thumbDiameterPx).coerceAtLeast(0f)
        if (isEmpty) {
            dragOffsetPx = (rawFraction - 0.5f) * usableWidth
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

        dragOffsetPx = (clampedResult.fraction - baseFraction) * usableWidth
        if (clampedResult.isAtSellLimit) {
            if (!atSellLimit) {
                atSellLimit = true
                haptics.warning()
            }
        } else {
            atSellLimit = false
        }

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
                haptics.tick()
            }
        }
    }

    fun onTap(offset: Offset) {
        if (isEmpty) {
            haptics.tick()
            onEmptyInteraction?.invoke()
        }
    }

    fun onDragEnd() {
        if (!isDragging) {
            dragOffsetPx = 0f
            accumulatedTranslationX = 0f
            return
        }
        isDragging = false

        if (isEmpty) {
            haptics.tick()
            showDepositPrompt = true
            depositPromptJob?.cancel()
            depositPromptJob = scope.launch {
                delay(2500)
                showDepositPrompt = false
            }
            triggerSnapBack(dragOffsetPx)
            return
        }

        val usableWidth = (barWidthPx - thumbDiameterPx).coerceAtLeast(0f)
        val targetFraction =
            if (usableWidth > 0f) {
                (baseFraction + (dragOffsetPx / usableWidth)).coerceIn(0f, 1f)
            } else {
                baseFraction
            }
        val evaluation =
            BalanceBarTradeCalculator.calculateSelection(
                initialFraction = baseFraction,
                targetFraction = targetFraction,
                totalUSD = totalUSD,
                stableUSD = stableUSD,
                maxSellUSD = maxSellUSD,
            )

        if (evaluation.isValidTrade && evaluation.direction != null && onTradeRequest != null) {
            haptics.impact()
            val request =
                TradeRequest(direction = evaluation.direction, amountUSD = evaluation.clampedUSD)
            onTradeRequest?.invoke(request)
        } else {
            triggerSnapBack(dragOffsetPx)
        }
    }

    fun onDragCancel() {
        isDragging = false
        triggerSnapBack(dragOffsetPx)
    }
}
