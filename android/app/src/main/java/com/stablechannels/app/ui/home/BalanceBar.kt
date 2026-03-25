package com.stablechannels.app.ui.home

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

enum class TradeDirection { BUY, SELL }

@Composable
fun BalanceBar(
    stableUSD: Double,
    nativeSats: Long,
    totalSats: Long,
    btcPrice: Double,
    modifier: Modifier = Modifier,
    onTradeRequest: ((TradeDirection, Double) -> Unit)? = null
) {
    val nativeUSD = (nativeSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val totalUSD = stableUSD + nativeUSD
    if (totalUSD <= 0) return

    val stableFraction = (stableUSD / totalUSD).coerceIn(0.0, 1.0).toFloat()
    val interactive = onTradeRequest != null
    val barHeight = if (interactive) 20.dp else 12.dp
    val thumbDiameter = 28.dp
    val minTradeUSD = 1.0

    var barWidthPx by remember { mutableFloatStateOf(0f) }
    var dragOffsetPx by remember { mutableFloatStateOf(0f) }
    var isDragging by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current

    val thumbDiameterPx = with(density) { thumbDiameter.toPx() }

    val baseXPx = barWidthPx * stableFraction
    val thumbXPx = (baseXPx + dragOffsetPx).coerceIn(0f, barWidthPx)
    val visFrac = if (barWidthPx > 0) thumbXPx / barWidthPx else stableFraction
    val usdPct = (visFrac * 100).roundToInt()
    val btcPct = 100 - usdPct

    val stableColor = Color(0xFF10B981)
    val nativeColor = Color(0xFFF59E0B)

    // Pulse animation for thumb
    val pulseScale = remember { Animatable(1f) }
    if (interactive) {
        LaunchedEffect(Unit) {
            pulseScale.animateTo(
                targetValue = 1.08f,
                animationSpec = infiniteRepeatable(
                    animation = tween(1500, easing = EaseInOut),
                    repeatMode = RepeatMode.Reverse
                )
            )
        }
    }

    Column(modifier = modifier.fillMaxWidth()) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(if (interactive) thumbDiameter else barHeight)
                .onSizeChanged { barWidthPx = it.width.toFloat() }
                .then(
                    if (interactive) {
                        Modifier.pointerInput(stableFraction) {
                            detectDragGestures(
                                onDragStart = { offset ->
                                    if (abs(offset.x - baseXPx) < thumbDiameterPx * 1.5f) {
                                        isDragging = true
                                        dragOffsetPx = 0f
                                    }
                                },
                                onDrag = { change, dragAmount ->
                                    if (isDragging) {
                                        change.consume()
                                        dragOffsetPx = (dragOffsetPx + dragAmount.x)
                                            .coerceIn(-baseXPx, barWidthPx - baseXPx)
                                    }
                                },
                                onDragEnd = {
                                    if (!isDragging) {
                                        dragOffsetPx = 0f
                                        return@detectDragGestures
                                    }
                                    isDragging = false
                                    val fraction = if (barWidthPx > 0) dragOffsetPx / barWidthPx else 0f
                                    val tradeUSD = abs(fraction) * totalUSD
                                    if (tradeUSD < minTradeUSD) {
                                        dragOffsetPx = 0f
                                        return@detectDragGestures
                                    }
                                    val direction = if (dragOffsetPx > 0) TradeDirection.SELL else TradeDirection.BUY
                                    val clamped = if (direction == TradeDirection.BUY)
                                        min(tradeUSD, stableUSD)
                                    else
                                        min(tradeUSD, nativeUSD)
                                    onTradeRequest?.invoke(direction, clamped)
                                    // Hold position, then snap back after sheet appears
                                    scope.launch {
                                        delay(600)
                                        dragOffsetPx = 0f
                                    }
                                },
                                onDragCancel = {
                                    isDragging = false
                                    dragOffsetPx = 0f
                                }
                            )
                        }
                    } else Modifier
                ),
            contentAlignment = Alignment.CenterStart
        ) {
            // Bar segments
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(barHeight)
                    .clip(RoundedCornerShape(6.dp))
            ) {
                if (visFrac > 0.01f) {
                    Box(
                        Modifier
                            .weight(max(visFrac, 0.03f))
                            .fillMaxHeight()
                            .background(Brush.horizontalGradient(listOf(stableColor.copy(alpha = 0.8f), stableColor)))
                    )
                }
                if ((1 - visFrac) > 0.01f) {
                    Box(
                        Modifier
                            .weight(max(1f - visFrac, 0.03f))
                            .fillMaxHeight()
                            .background(Brush.horizontalGradient(listOf(nativeColor, nativeColor.copy(alpha = 0.8f))))
                    )
                }
            }

            // Draggable thumb
            if (interactive && barWidthPx > 0) {
                val thumbOffsetDp = with(density) { thumbXPx.toDp() } - thumbDiameter / 2
                Box(
                    modifier = Modifier
                        .offset(x = thumbOffsetDp)
                        .size(thumbDiameter)
                        .scale(if (isDragging) 1.15f else pulseScale.value)
                        .shadow(4.dp, CircleShape)
                        .clip(CircleShape)
                        .background(Color.White)
                )

                // Percentage label while dragging
                if (isDragging) {
                    Box(
                        modifier = Modifier
                            .offset(x = thumbOffsetDp - 30.dp, y = (-34).dp)
                            .background(
                                MaterialTheme.colorScheme.surfaceVariant,
                                RoundedCornerShape(12.dp)
                            )
                            .padding(horizontal = 8.dp, vertical = 4.dp)
                    ) {
                        Text(
                            "$usdPct% USD  $btcPct% BTC",
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }
            }
        }

        Spacer(Modifier.height(4.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text(
                "${stableUSD.usdFormatted()} stable",
                style = MaterialTheme.typography.labelSmall,
                color = stableColor
            )
            Text(
                "${nativeUSD.usdFormatted()} native",
                style = MaterialTheme.typography.labelSmall,
                color = nativeColor
            )
        }
    }
}
