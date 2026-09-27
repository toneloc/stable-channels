package com.stablechannels.app.ui.home

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.models.PriceRecord
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@Composable
fun PriceChartGraph(
    priceHistory: List<PriceRecord>,
    chartPeriod: ChartPeriod,
    currentPrice: Double,
    selectedPoint: PriceRecord?,
    onPointSelected: (PriceRecord?) -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptic = LocalHapticFeedback.current

    if (priceHistory.size < 2) {
        PriceChartLoadingView(modifier = modifier)
        return
    }

    val (minPrice, maxPrice) =
        remember(priceHistory) { PriceChartAlgorithms.minMaxPrices(priceHistory) }
    val priceRange = maxPrice - minPrice
    val firstPrice = priceHistory.first().price
    val displayPrice = selectedPoint?.price ?: currentPrice
    val isUp = displayPrice >= firstPrice
    val lineColor = if (isUp) Color(0xFF10B981) else Color(0xFFEF4444)

    val selectedIndex = selectedPoint?.let { sp ->
        priceHistory.indexOfFirst { it.id == sp.id }.takeIf { it >= 0 }
    }

    // Chart with Y-axis labels
    Row(modifier = modifier.fillMaxWidth()) {
        Canvas(
            modifier =
                Modifier.weight(1f)
                    .height(160.dp)
                    .pointerInput(priceHistory) {
                        detectDragGestures(
                            onDragEnd = { onPointSelected(null) },
                            onDragCancel = { onPointSelected(null) },
                            onDrag = { change, _ ->
                                change.consume()
                                val x = change.position.x
                                val w = size.width.toFloat()
                                val index =
                                    ((x / w) * (priceHistory.size - 1))
                                        .toInt()
                                        .coerceIn(0, priceHistory.size - 1)
                                val record = priceHistory[index]
                                if (selectedPoint?.id != record.id) {
                                    onPointSelected(record)
                                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                }
                            },
                        )
                    }
                    .pointerInput(priceHistory) {
                        detectTapGestures(
                            onPress = {
                                val x = it.x
                                val w = size.width.toFloat()
                                val index =
                                    ((x / w) * (priceHistory.size - 1))
                                        .toInt()
                                        .coerceIn(0, priceHistory.size - 1)
                                val record = priceHistory[index]
                                if (selectedPoint?.id != record.id) {
                                    onPointSelected(record)
                                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                }
                                tryAwaitRelease()
                                onPointSelected(null)
                            }
                        )
                    }
        ) {
            val w = size.width
            val h = size.height

            if (priceRange < 0.01) {
                drawLine(
                    color = lineColor,
                    start = Offset(0f, h / 2),
                    end = Offset(w, h / 2),
                    strokeWidth = 2f,
                )
                return@Canvas
            }

            // Grid lines
            for (i in 1..3) {
                val gy = h * i / 4
                drawLine(
                    color = Color.Gray.copy(alpha = 0.15f),
                    start = Offset(0f, gy),
                    end = Offset(w, gy),
                    strokeWidth = 0.5f,
                    pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 8f)),
                )
            }

            // Project points to canvas dimensions
            val points = priceHistory.mapIndexed { i, record ->
                val px = (i.toFloat() / (priceHistory.size - 1)) * w
                val py = h - ((record.price - minPrice) / priceRange).toFloat() * h
                Offset(px, py)
            }

            val linePath = PriceChartAlgorithms.buildSplinePath(points)
            val areaPath = PriceChartAlgorithms.buildAreaPath(linePath, w, h)

            // Area fill
            drawPath(
                path = areaPath,
                brush =
                    Brush.verticalGradient(
                        colors =
                            listOf(
                                lineColor.copy(alpha = 0.15f),
                                lineColor.copy(alpha = 0.02f),
                            )
                    ),
                style = Fill,
            )

            // Line
            drawPath(
                path = linePath,
                color = lineColor,
                style = Stroke(width = if (selectedIndex != null) 1.5f else 2f),
            )

            // Selected indicator
            if (selectedIndex != null) {
                val sx = (selectedIndex.toFloat() / (priceHistory.size - 1)) * w
                val record = priceHistory[selectedIndex]
                val sy = h - ((record.price - minPrice) / priceRange).toFloat() * h
                drawLine(
                    color = Color.Gray.copy(alpha = 0.5f),
                    start = Offset(sx, 0f),
                    end = Offset(sx, h),
                    strokeWidth = 1f,
                    pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 6f)),
                )
                drawCircle(lineColor, 5f, Offset(sx, sy))
                drawCircle(Color.White, 3f, Offset(sx, sy))
            }
        }

        // Y-axis labels
        Column(
            modifier = Modifier.height(160.dp).padding(start = 4.dp),
            verticalArrangement = Arrangement.SpaceBetween,
            horizontalAlignment = Alignment.End,
        ) {
            for (i in 4 downTo 0) {
                val price = minPrice + (priceRange * i / 4)
                Text(
                    PriceChartAlgorithms.formatYAxis(price),
                    fontSize = 9.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }

    Spacer(Modifier.height(4.dp))

    // X-axis time labels
    val xFmt =
        when {
            chartPeriod == ChartPeriod.DAY_1 -> SimpleDateFormat("ha", Locale.US)
            chartPeriod.effectiveDays() <= 90 -> SimpleDateFormat("MMM d", Locale.US)
            chartPeriod.effectiveDays() <= 365 -> SimpleDateFormat("MMM", Locale.US)
            else -> SimpleDateFormat("yyyy", Locale.US)
        }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        val step = maxOf(priceHistory.size / 4, 1)
        for (i in listOf(0, step, step * 2, step * 3, priceHistory.size - 1).distinct()) {
            if (i < priceHistory.size) {
                Text(
                    xFmt.format(Date(priceHistory[i].timestamp * 1000)),
                    fontSize = 9.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
