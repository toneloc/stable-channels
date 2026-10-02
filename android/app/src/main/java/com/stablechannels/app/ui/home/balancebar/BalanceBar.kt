package com.stablechannels.app.ui.home.balancebar

import android.view.HapticFeedbackConstants
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.EaseOut
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import com.stablechannels.app.util.Constants
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun BalanceBar(
    stableUSD: Double,
    nativeSats: Long,
    totalSats: Long,
    btcPrice: Double,
    maxSellUSD: Double = 0.0,
    isTrading: Boolean = false,
    showBtcFormat: Boolean = false,
    modifier: Modifier = Modifier,
    onDragStarted: (() -> Unit)? = null,
    onTradeRequest: ((TradeDirection, Double) -> Unit)? = null,
    onEmptyInteraction: (() -> Unit)? = null,
) {
    val nativeUSD = (nativeSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val totalUSD = stableUSD + nativeUSD
    val isEmpty = totalUSD <= 0.0 || totalSats <= 0L

    val canonicalFraction =
        if (totalUSD > 0.0) (stableUSD / totalUSD).coerceIn(0.0, 1.0).toFloat() else 0.5f

    val interactive = onTradeRequest != null || onEmptyInteraction != null
    val barHeight = if (interactive) 12.dp else 8.dp
    val thumbDiameter = 22.dp
    val minTradeUSD = 1.0

    var barWidthPx by remember { mutableFloatStateOf(0f) }
    var isDragging by remember { mutableStateOf(false) }
    var isSnappingBack by remember { mutableStateOf(false) }
    var hasTriggeredHaptic by remember { mutableStateOf(false) }
    var atSellLimit by remember { mutableStateOf(false) }
    var showDepositPrompt by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val view = LocalView.current

    var dragOffsetPx by remember { mutableFloatStateOf(0f) }
    val snapBackAnim = remember { Animatable(0f) }
    val animator = rememberBalanceBarAnimationCoordinator()

    fun triggerSnapBack(fromOffset: Float) {
        isSnappingBack = true
        scope.launch {
            snapBackAnim.snapTo(fromOffset)
            snapBackAnim.animateTo(0f, tween(350, easing = EaseOut))
            dragOffsetPx = 0f
            isSnappingBack = false
        }
    }

    var wasEmpty by remember { mutableStateOf(isEmpty) }
    LaunchedEffect(isEmpty) {
        if (wasEmpty && !isEmpty) {
            animator.triggerAwakening(canonicalFraction)
        }
        wasEmpty = isEmpty
    }

    var wasTrading by remember { mutableStateOf(false) }
    LaunchedEffect(isTrading) {
        if (wasTrading && !isTrading) {
            triggerSnapBack(dragOffsetPx)
        }
        wasTrading = isTrading
    }

    LaunchedEffect(canonicalFraction) {
        if (!isTrading && !isDragging && dragOffsetPx != 0f) {
            triggerSnapBack(dragOffsetPx)
        }
    }

    val currentFraction =
        when {
            animator.isAwakening && animator.settleFraction != null -> animator.settleFraction!!
            isEmpty -> 0.5f
            else -> canonicalFraction
        }

    val currentOffsetPx = if (isSnappingBack) snapBackAnim.value else dragOffsetPx
    val thumbDiameterPx = with(density) { thumbDiameter.toPx() }
    val baseXPx = barWidthPx * currentFraction
    val maxSellOffset =
        if (totalUSD > 0.0) {
            minOf(
                (barWidthPx * maxSellUSD.coerceAtLeast(0.0) / totalUSD).toFloat(),
                (barWidthPx - baseXPx).coerceAtLeast(0f),
            )
        } else 0f

    val thumbXPx = (baseXPx + currentOffsetPx).coerceIn(0f, barWidthPx)
    val visFrac = if (barWidthPx > 0) (thumbXPx / barWidthPx).coerceIn(0f, 1f) else currentFraction
    val usdPct = (visFrac * 100).roundToInt()
    val btcPct = 100 - usdPct

    val stableColor = Color(0xFF10B981)
    val nativeColor = Color(0xFFF59E0B)

    val pulseScale = remember { Animatable(1f) }
    if (interactive && !isEmpty) {
        LaunchedEffect(Unit) {
            pulseScale.animateTo(
                targetValue = 1.08f,
                animationSpec =
                    infiniteRepeatable(
                        animation = tween(1500, easing = EaseInOut),
                        repeatMode = RepeatMode.Reverse,
                    ),
            )
        }
    }

    Column(modifier = modifier.fillMaxWidth()) {
        if (interactive) {
            val showConversion = isDragging || abs(currentOffsetPx) > 0.5f || animator.isAwakening
            BalanceBarHeader(
                usdPct = usdPct,
                btcPct = btcPct,
                atSellLimit = atSellLimit,
                maxSellUSD = maxSellUSD,
                showConversion = showConversion,
                showDepositPrompt = showDepositPrompt,
                stableColor = stableColor,
                nativeColor = nativeColor,
            )
            Spacer(Modifier.height(4.dp))
        }

        Box(
            modifier =
                Modifier.fillMaxWidth()
                    .height(if (interactive) thumbDiameter else barHeight)
                    .onSizeChanged { barWidthPx = it.width.toFloat() }
                    .then(
                        if (interactive && !animator.isAwakening) {
                            Modifier.pointerInput(currentFraction, maxSellUSD, totalUSD, isEmpty) {
                                detectDragGestures(
                                    onDragStart = { offset ->
                                        if (isEmpty) {
                                            view.performHapticFeedback(
                                                HapticFeedbackConstants.CLOCK_TICK
                                            )
                                            showDepositPrompt = true
                                            onEmptyInteraction?.invoke()
                                            scope.launch {
                                                delay(1800)
                                                showDepositPrompt = false
                                            }
                                            return@detectDragGestures
                                        }
                                        if (abs(offset.x - baseXPx) < thumbDiameterPx * 1.5f) {
                                            isDragging = true
                                            isSnappingBack = false
                                            hasTriggeredHaptic = false
                                            atSellLimit = false
                                            dragOffsetPx = 0f
                                            onDragStarted?.invoke()
                                            view.performHapticFeedback(
                                                HapticFeedbackConstants.CLOCK_TICK
                                            )
                                        }
                                    },
                                    onDrag = { change, dragAmount ->
                                        if (isEmpty) return@detectDragGestures
                                        if (isDragging) {
                                            change.consume()
                                            val proposedOffset = dragOffsetPx + dragAmount.x
                                            atSellLimit = proposedOffset > maxSellOffset
                                            val newOffset =
                                                proposedOffset.coerceIn(-baseXPx, maxSellOffset)
                                            dragOffsetPx = newOffset

                                            if (!hasTriggeredHaptic && barWidthPx > 0) {
                                                val fraction = abs(newOffset) / barWidthPx
                                                val tradeUSD = fraction * totalUSD
                                                if (tradeUSD >= minTradeUSD) {
                                                    hasTriggeredHaptic = true
                                                    view.performHapticFeedback(
                                                        HapticFeedbackConstants.CLOCK_TICK
                                                    )
                                                }
                                            }
                                        }
                                    },
                                    onDragEnd = {
                                        if (isEmpty || !isDragging) {
                                            dragOffsetPx = 0f
                                            return@detectDragGestures
                                        }
                                        isDragging = false
                                        val offset = dragOffsetPx
                                        val fraction =
                                            if (barWidthPx > 0) offset / barWidthPx else 0f
                                        val tradeUSD = abs(fraction) * totalUSD
                                        if (tradeUSD < minTradeUSD) {
                                            triggerSnapBack(offset)
                                            return@detectDragGestures
                                        }
                                        val direction =
                                            if (offset > 0) TradeDirection.SELL
                                            else TradeDirection.BUY
                                        val clamped =
                                            if (direction == TradeDirection.BUY)
                                                min(tradeUSD, stableUSD)
                                            else min(tradeUSD, maxSellUSD)
                                        onTradeRequest?.invoke(direction, clamped)
                                    },
                                    onDragCancel = {
                                        isDragging = false
                                        triggerSnapBack(dragOffsetPx)
                                    },
                                )
                            }
                        } else Modifier
                    ),
            contentAlignment = Alignment.CenterStart,
        ) {
            BalanceBarAwakeningBloom(
                barWidthPx = barWidthPx,
                floodScale = animator.radialFloodScale.value,
                floodAlpha = animator.radialFloodAlpha.value,
                nativeColor = nativeColor,
                modifier = Modifier.align(Alignment.Center),
            )

            BalanceBarTrack(
                visFrac = visFrac,
                barHeight = barHeight,
                isEmpty = isEmpty,
                isAwakening = animator.isAwakening,
                stableColor = stableColor,
                nativeColor = nativeColor,
            )

            if (interactive && barWidthPx > 0) {
                val thumbOffsetDp = with(density) { thumbXPx.toDp() } - thumbDiameter / 2
                val currentScale =
                    when {
                        animator.isAwakening -> animator.thumbAwakenScale.value
                        isDragging -> 1.15f
                        isEmpty -> 1.0f
                        else -> pulseScale.value
                    }
                BalanceBarThumb(
                    thumbOffsetDp = thumbOffsetDp,
                    thumbDiameter = thumbDiameter,
                    scale = currentScale,
                )
            }
        }

        Spacer(Modifier.height(6.dp))
        BalanceBarLabels(
            stableUSD = stableUSD,
            nativeSats = nativeSats,
            nativeUSD = nativeUSD,
            btcPrice = btcPrice,
            showBtcFormat = showBtcFormat,
            isEmpty = isEmpty,
            stableColor = stableColor,
            nativeColor = nativeColor,
        )
    }
}
