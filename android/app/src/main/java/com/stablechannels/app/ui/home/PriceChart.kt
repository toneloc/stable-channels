package com.stablechannels.app.ui.home

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.models.PriceRecord
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.abs

enum class ChartPeriod(val label: String, val hours: Int) {
    DAY_1("1D", 24),
    WEEK_1("1W", 168),
    MONTH_1("1M", 720),
    YEAR_1("1Y", 8760),
    YEAR_3("3Y", 26280),
    ALL("ALL", Int.MAX_VALUE)
}

@Composable
fun PriceChart(
    databaseService: DatabaseService?,
    currentPrice: Double,
    modifier: Modifier = Modifier
) {
    var chartPeriod by remember { mutableStateOf(ChartPeriod.ALL) }
    var priceHistory by remember { mutableStateOf(emptyList<PriceRecord>()) }
    var selectedPoint by remember { mutableStateOf<PriceRecord?>(null) }

    LaunchedEffect(chartPeriod, currentPrice) {
        selectedPoint = null
        withContext(Dispatchers.IO) {
            priceHistory = when (chartPeriod) {
                ChartPeriod.DAY_1, ChartPeriod.WEEK_1, ChartPeriod.MONTH_1 -> {
                    databaseService?.getPriceHistory(chartPeriod.hours) ?: emptyList()
                }
                else -> {
                    val days = when (chartPeriod) {
                        ChartPeriod.YEAR_1 -> 365
                        ChartPeriod.YEAR_3 -> 1095
                        else -> 99999
                    }
                    val dailyPrices = databaseService?.getDailyPrices(days) ?: emptyList()
                    val fmt = SimpleDateFormat("yyyy-MM-dd", Locale.US)
                    dailyPrices.mapNotNull { daily ->
                        val date = fmt.parse(daily.date) ?: return@mapNotNull null
                        val ts = date.time / 1000
                        PriceRecord(id = ts, price = daily.close, source = "daily", timestamp = ts)
                    }.sortedBy { it.timestamp }
                }
            }
        }
    }

    Card(
        modifier = modifier.fillMaxWidth(),
        colors = androidx.compose.material3.CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.background
        )
    ) {
        Column(Modifier.padding(16.dp)) {
            // Price header — shows selected point or current price
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Column {
                    val selected = selectedPoint
                    if (selected != null) {
                        val dateFmt = if (chartPeriod == ChartPeriod.DAY_1) {
                            SimpleDateFormat("h:mm a", Locale.US)
                        } else {
                            SimpleDateFormat("MMM. d, yyyy", Locale.US)
                        }
                        Text(
                            dateFmt.format(Date(selected.timestamp * 1000)),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            selected.price.usdFormatted(),
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.Bold
                        )
                    } else {
                        Text("BTC Price", style = MaterialTheme.typography.labelMedium)
                        Text(
                            currentPrice.usdFormatted(),
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }
                if (priceHistory.size >= 2) {
                    val displayPrice = selectedPoint?.price ?: currentPrice
                    val firstPrice = priceHistory.first().price
                    val isUp = displayPrice >= firstPrice
                    val lineColor = if (isUp) Color(0xFF10B981) else Color(0xFFEF4444)
                    val changePercent = if (firstPrice > 0) ((displayPrice - firstPrice) / firstPrice) * 100 else 0.0

                    Column(horizontalAlignment = Alignment.End) {
                        Text(
                            chartPeriod.label,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            String.format("%+.2f%%", changePercent),
                            color = lineColor,
                            fontWeight = FontWeight.SemiBold,
                            fontSize = 14.sp
                        )
                    }
                }
            }

            Spacer(Modifier.height(8.dp))

            // Period selector pills
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                ChartPeriod.entries.forEach { period ->
                    val selected = chartPeriod == period
                    Surface(
                        onClick = { chartPeriod = period },
                        shape = CircleShape,
                        color = if (selected) Color(0xFF3B82F6) else Color.Transparent,
                    ) {
                        Text(
                            period.label,
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold,
                            color = if (selected) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp),
                            textAlign = TextAlign.Center
                        )
                    }
                }
            }

            Spacer(Modifier.height(12.dp))

            if (priceHistory.size >= 2) {
                val prices = priceHistory.map { it.price }
                val minPrice = prices.min()
                val maxPrice = prices.max()
                val priceRange = maxPrice - minPrice
                val firstPrice = prices.first()
                val displayPrice = selectedPoint?.price ?: currentPrice
                val isUp = displayPrice >= firstPrice
                val lineColor = if (isUp) Color(0xFF10B981) else Color(0xFFEF4444)

                // Find selected index for drawing the indicator
                val selectedIndex = selectedPoint?.let { sp ->
                    priceHistory.indexOfFirst { it.id == sp.id }.takeIf { it >= 0 }
                }

                Canvas(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(150.dp)
                        .pointerInput(priceHistory) {
                            detectDragGestures(
                                onDragEnd = { selectedPoint = null },
                                onDragCancel = { selectedPoint = null },
                                onDrag = { change, _ ->
                                    change.consume()
                                    val x = change.position.x
                                    val w = size.width.toFloat()
                                    val index = ((x / w) * (priceHistory.size - 1))
                                        .toInt()
                                        .coerceIn(0, priceHistory.size - 1)
                                    selectedPoint = priceHistory[index]
                                }
                            )
                        }
                        .pointerInput(priceHistory) {
                            detectTapGestures(
                                onPress = {
                                    val x = it.x
                                    val w = size.width.toFloat()
                                    val index = ((x / w) * (priceHistory.size - 1))
                                        .toInt()
                                        .coerceIn(0, priceHistory.size - 1)
                                    selectedPoint = priceHistory[index]
                                    tryAwaitRelease()
                                    selectedPoint = null
                                }
                            )
                        }
                ) {
                    val w = size.width
                    val h = size.height
                    val padding = 4f

                    if (priceRange < 0.01) {
                        drawLine(
                            color = lineColor,
                            start = Offset(0f, h / 2),
                            end = Offset(w, h / 2),
                            strokeWidth = 2f
                        )
                        return@Canvas
                    }

                    // Draw line
                    val path = Path()
                    priceHistory.forEachIndexed { i, record ->
                        val px = (i.toFloat() / (priceHistory.size - 1)) * w
                        val py = h - padding - ((record.price - minPrice) / priceRange).toFloat() * (h - padding * 2)
                        if (i == 0) path.moveTo(px, py) else path.lineTo(px, py)
                    }

                    drawPath(
                        path = path,
                        color = lineColor,
                        style = Stroke(width = if (selectedIndex != null) 1.5f else 2.5f)
                    )

                    // Draw selected indicator
                    if (selectedIndex != null) {
                        val sx = (selectedIndex.toFloat() / (priceHistory.size - 1)) * w
                        val record = priceHistory[selectedIndex]
                        val sy = h - padding - ((record.price - minPrice) / priceRange).toFloat() * (h - padding * 2)

                        // Vertical dashed line
                        drawLine(
                            color = Color.Gray.copy(alpha = 0.5f),
                            start = Offset(sx, 0f),
                            end = Offset(sx, h),
                            strokeWidth = 1f,
                            pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 6f))
                        )

                        // Dot
                        drawCircle(
                            color = lineColor,
                            radius = 5f,
                            center = Offset(sx, sy)
                        )
                        drawCircle(
                            color = Color.White,
                            radius = 3f,
                            center = Offset(sx, sy)
                        )
                    }
                }

                Spacer(Modifier.height(4.dp))

                // Time labels
                val isDaily = chartPeriod.hours > 720
                val fmt = if (isDaily) {
                    SimpleDateFormat("MMM. yyyy", Locale.US)
                } else if (chartPeriod == ChartPeriod.DAY_1) {
                    SimpleDateFormat("h:mm a", Locale.US)
                } else {
                    SimpleDateFormat("MMM. d", Locale.US)
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Text(
                        fmt.format(Date(priceHistory.first().timestamp * 1000)),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Text(
                        fmt.format(Date(priceHistory.last().timestamp * 1000)),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            } else {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(150.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        "Collecting price data...",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }
    }
}
