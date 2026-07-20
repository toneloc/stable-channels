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
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import android.view.HapticFeedbackConstants
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material3.Icon
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted
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
    showBtcFormat: Boolean = false,
    modifier: Modifier = Modifier,
    onDragStarted: (() -> Unit)? = null,
    onTradeRequest: ((TradeDirection, Double) -> Unit)? = null
) {
    val nativeUSD = (nativeSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val totalUSD = stableUSD + nativeUSD
    if (totalUSD <= 0) return

    val stableFraction = (stableUSD / totalUSD).coerceIn(0.0, 1.0).toFloat()
    val interactive = onTradeRequest != null
    val barHeight = if (interactive) 12.dp else 8.dp
    val thumbDiameter = 22.dp
    val minTradeUSD = 1.0

    var barWidthPx by remember { mutableFloatStateOf(0f) }
    var isDragging by remember { mutableStateOf(false) }
    var hasTriggeredHaptic by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val view = LocalView.current

    // Use Animatable for smooth snap-back animation
    val dragOffset = remember { Animatable(0f) }
    val dragOffsetPx = dragOffset.value

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
                                        hasTriggeredHaptic = false
                                        scope.launch { dragOffset.snapTo(0f) }
                                        onDragStarted?.invoke()
                                    }
                                },
                                onDrag = { change, dragAmount ->
                                    if (isDragging) {
                                        change.consume()
                                        val newOffset = (dragOffset.value + dragAmount.x)
                                            .coerceIn(-baseXPx, barWidthPx - baseXPx)
                                        scope.launch { dragOffset.snapTo(newOffset) }

                                        // Haptic tick when drag first crosses $1.00 threshold
                                        if (!hasTriggeredHaptic && barWidthPx > 0) {
                                            val fraction = abs(newOffset) / barWidthPx
                                            val tradeUSD = fraction * totalUSD
                                            if (tradeUSD >= minTradeUSD) {
                                                hasTriggeredHaptic = true
                                                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                                            }
                                        }
                                    }
                                },
                                onDragEnd = {
                                    if (!isDragging) {
                                        scope.launch { dragOffset.snapTo(0f) }
                                        return@detectDragGestures
                                    }
                                    isDragging = false
                                    val currentOffset = dragOffset.value
                                    val fraction = if (barWidthPx > 0) currentOffset / barWidthPx else 0f
                                    val tradeUSD = abs(fraction) * totalUSD
                                    if (tradeUSD < minTradeUSD) {
                                        // Below threshold: animate snap-back (400ms ease-out)
                                        scope.launch {
                                            dragOffset.animateTo(
                                                targetValue = 0f,
                                                animationSpec = tween(400, easing = androidx.compose.animation.core.EaseOut)
                                            )
                                        }
                                        return@detectDragGestures
                                    }
                                    val direction = if (currentOffset > 0) TradeDirection.SELL else TradeDirection.BUY
                                    val clamped = if (direction == TradeDirection.BUY)
                                        min(tradeUSD, stableUSD)
                                    else
                                        min(tradeUSD, nativeUSD)
                                    onTradeRequest?.invoke(direction, clamped)
                                    // Hold position, then animate back (400ms ease-out)
                                    scope.launch {
                                        kotlinx.coroutines.delay(600)
                                        dragOffset.animateTo(
                                            targetValue = 0f,
                                            animationSpec = tween(400, easing = androidx.compose.animation.core.EaseOut)
                                        )
                                    }
                                },
                                onDragCancel = {
                                    isDragging = false
                                    scope.launch {
                                        dragOffset.animateTo(
                                            targetValue = 0f,
                                            animationSpec = tween(400, easing = androidx.compose.animation.core.EaseOut)
                                        )
                                    }
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
                    val barWidthDp = with(density) { barWidthPx.toDp() }
                    val labelWidth = 105.dp
                    val labelX = (thumbOffsetDp - (labelWidth / 2) + (thumbDiameter / 2))
                        .coerceIn(0.dp, maxOf(0.dp, barWidthDp - labelWidth))

                    val xPx = with(density) { labelX.roundToPx() }
                    val yPx = with(density) { (-40).dp.roundToPx() }

                    Popup(
                        alignment = Alignment.TopStart,
                        offset = IntOffset(xPx, yPx),
                        properties = PopupProperties(clippingEnabled = false)
                    ) {
                        Box(
                            modifier = Modifier
                                .background(
                                    MaterialTheme.colorScheme.surfaceVariant,
                                    RoundedCornerShape(12.dp)
                                )
                                .padding(horizontal = 8.dp, vertical = 6.dp)
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
        }

        Spacer(Modifier.height(6.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            // Left: USD label + amount
            val stableSats = if (btcPrice > 0) (stableUSD / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
            Column {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    Icon(Icons.Default.Shield, contentDescription = null, tint = stableColor, modifier = Modifier.size(12.dp))
                    Text("USD", style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.Bold, color = stableColor)
                }
                Text(
                    if (showBtcFormat) stableSats.btcSpacedFormatted() else stableUSD.usdFormatted(),
                    style = MaterialTheme.typography.labelSmall,
                    color = if (btcPrice > 0) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // Right: BTC label + amount
            Column(horizontalAlignment = Alignment.End) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("BTC", style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.Bold, color = nativeColor)
                    Icon(Icons.Default.CurrencyBitcoin, contentDescription = null, tint = nativeColor, modifier = Modifier.size(12.dp))
                }
                Text(
                    if (showBtcFormat) nativeSats.btcSpacedFormatted()
                    else if (btcPrice > 0) nativeUSD.usdFormatted() else "...",
                    style = MaterialTheme.typography.labelSmall,
                    color = if (btcPrice > 0) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}
