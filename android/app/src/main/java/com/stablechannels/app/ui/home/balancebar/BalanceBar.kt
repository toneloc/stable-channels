package com.stablechannels.app.ui.home.balancebar

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
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
import kotlin.math.roundToInt

object BalanceBarDefaults {
    val THUMB_DIAMETER = 22.dp

    fun isChannelEmpty(totalSats: Long, stableUSD: Double): Boolean =
        totalSats <= 0L && stableUSD <= 0.0
}

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
    onTradeRequest: ((TradeRequest) -> Unit)? = null,
    onEmptyInteraction: (() -> Unit)? = null,
) {
    val nativeUSD = (nativeSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val totalUSD = stableUSD + nativeUSD
    val isEmpty = BalanceBarDefaults.isChannelEmpty(totalSats = totalSats, stableUSD = stableUSD)

    val canonicalFraction =
        if (totalUSD > 0.0) (stableUSD / totalUSD).coerceIn(0.0, 1.0).toFloat() else 0.5f

    val isPriceReady = btcPrice > 0.0
    val interactive =
        (isEmpty && onEmptyInteraction != null) ||
            (!isEmpty && isPriceReady && onTradeRequest != null)
    val barHeight = 20.dp
    val thumbDiameter = BalanceBarDefaults.THUMB_DIAMETER

    var barWidthPx by remember { mutableFloatStateOf(0f) }
    val density = LocalDensity.current
    val view = LocalView.current
    val haptics = remember(view) { DefaultBalanceBarHaptics(view) }

    val state =
        rememberBalanceBarState(
            totalUSD = totalUSD,
            stableUSD = stableUSD,
            maxSellUSD = maxSellUSD,
            isEmpty = isEmpty,
            density = density.density,
            haptics = haptics,
            onDragStarted = onDragStarted,
            onTradeRequest = onTradeRequest,
            onEmptyInteraction = onEmptyInteraction,
        )

    val animator = rememberBalanceBarAnimationCoordinator()

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
            state.triggerSnapBack(state.dragOffsetPx)
        }
        wasTrading = isTrading
    }

    LaunchedEffect(canonicalFraction) {
        if (!isTrading && !state.isDragging && state.dragOffsetPx != 0f) {
            state.triggerSnapBack(state.dragOffsetPx)
        }
    }

    val currentFraction =
        when {
            animator.isAwakening && animator.settleFraction != null -> animator.settleFraction!!
            isEmpty -> 0.5f
            else -> canonicalFraction
        }

    val thumbDiameterPx = with(density) { thumbDiameter.toPx() }
    val thumbRadiusPx = thumbDiameterPx / 2f
    SideEffect { state.updateLayout(barWidthPx, thumbDiameterPx) }

    val baseXPx =
        BalanceBarTradeCalculator.calculateThumbPosition(
            fraction = currentFraction,
            barWidth = barWidthPx,
            thumbDiameter = thumbDiameterPx,
        )

    val minThumbX = thumbRadiusPx
    val maxThumbX = (barWidthPx - thumbRadiusPx).coerceAtLeast(thumbRadiusPx)
    val thumbXPx = (baseXPx + state.currentOffsetPx).coerceIn(minThumbX, maxThumbX)

    val usableWidthPx = (barWidthPx - thumbDiameterPx).coerceAtLeast(0f)
    val activeFraction =
        if (usableWidthPx > 0f) ((thumbXPx - thumbRadiusPx) / usableWidthPx).coerceIn(0f, 1f)
        else currentFraction
    val usdPct = (activeFraction * 100).roundToInt()
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
            val showConversion =
                state.isDragging ||
                    abs(state.currentOffsetPx) > 0.5f ||
                    animator.isAwakening ||
                    state.showDepositPrompt
            BalanceBarHeader(
                usdPct = usdPct,
                btcPct = btcPct,
                atSellLimit = state.atSellLimit,
                maxSellUSD = maxSellUSD,
                showConversion = showConversion,
                showDepositPrompt = state.showDepositPrompt,
                stableColor = stableColor,
                nativeColor = nativeColor,
                onEmptyInteraction = onEmptyInteraction,
            )
            Spacer(Modifier.height(4.dp))
        }

        Box(
            modifier =
                Modifier.fillMaxWidth()
                    .height(if (interactive) thumbDiameter else barHeight)
                    .onSizeChanged {
                        barWidthPx = it.width.toFloat()
                        state.updateLayout(it.width.toFloat(), thumbDiameterPx)
                    }
                    .then(
                        if (interactive && !animator.isAwakening) {
                            Modifier.pointerInput(Unit) {
                                    detectTapGestures(
                                        onTap = { offset ->
                                            state.onTap(offset)
                                        }
                                    )
                                }
                                .pointerInput(Unit) {
                                    detectDragGestures(
                                        onDragStart = { offset ->
                                            state.onDragStart(offset)
                                        },
                                        onDrag = { change, dragAmount ->
                                            change.consume()
                                            state.onDrag(dragAmount.x)
                                        },
                                        onDragEnd = { state.onDragEnd() },
                                        onDragCancel = { state.onDragCancel() },
                                    )
                                }
                        } else Modifier
                    ),
            contentAlignment = Alignment.CenterStart,
        ) {
            BalanceBarAwakeningBloom(
                barWidthPx = barWidthPx,
                floodScale = animator.radialFloodScale,
                floodAlpha = animator.radialFloodAlpha,
                nativeColor = nativeColor,
                modifier = Modifier.align(Alignment.Center),
            )

            BalanceBarTrack(
                fraction = activeFraction,
                thumbXPx = thumbXPx,
                barWidthPx = barWidthPx,
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
                        animator.isAwakening -> animator.thumbAwakenScale
                        state.isDragging -> 1.15f
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
