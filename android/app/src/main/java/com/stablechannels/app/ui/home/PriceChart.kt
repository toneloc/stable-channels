package com.stablechannels.app.ui.home

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Fill
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
import java.util.Calendar
import java.util.Date
import java.util.Locale
import kotlin.math.abs

enum class ChartPeriod(val label: String, val days: Int, val usesHourly: Boolean) {
    DAY_1("1D", 1, true),
    WEEK_1("1W", 7, true),
    MONTH_1("1M", 30, true),
    MONTH_3("3M", 90, false),
    MONTH_6("6M", 180, false),
    YTD("YTD", -1, false),  // computed dynamically
    YEAR_1("1Y", 365, false),
    YEAR_2("2Y", 730, false),
    YEAR_5("5Y", 1825, false),
    YEAR_10("10Y", 3650, false),
    ALL("ALL", 99999, false);

    fun effectiveDays(): Int {
        if (this == YTD) {
            val cal = Calendar.getInstance()
            val dayOfYear = cal.get(Calendar.DAY_OF_YEAR)
            return dayOfYear
        }
        return days
    }
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

    // Cached data — loaded once
    var allDailyPrices by remember { mutableStateOf(emptyList<PriceRecord>()) }
    var hourlyPrices by remember { mutableStateOf(emptyList<PriceRecord>()) }
    var dataLoaded by remember { mutableStateOf(false) }

    // Load all data once
    LaunchedEffect(Unit) {
        if (dataLoaded) return@LaunchedEffect
        withContext(Dispatchers.IO) {
            hourlyPrices = databaseService?.getPriceHistory(24 * 30) ?: emptyList()

            val dailyPrices = databaseService?.getDailyPrices(99999) ?: emptyList()
            val fmt = SimpleDateFormat("yyyy-MM-dd", Locale.US)
            allDailyPrices = dailyPrices.mapNotNull { daily ->
                val date = fmt.parse(daily.date) ?: return@mapNotNull null
                val ts = date.time / 1000
                PriceRecord(id = ts, price = daily.close, source = "daily", timestamp = ts)
            }.sortedBy { it.timestamp }
        }
        dataLoaded = true
    }

    // Filter when period changes or data loads
    LaunchedEffect(chartPeriod, dataLoaded) {
        if (!dataLoaded) return@LaunchedEffect
        selectedPoint = null
        val cutoffMs = System.currentTimeMillis() - chartPeriod.effectiveDays().toLong() * 86400 * 1000
        val cutoffSec = cutoffMs / 1000
        priceHistory = if (chartPeriod.usesHourly) {
            hourlyPrices.filter { it.timestamp >= cutoffSec }
        } else {
            allDailyPrices.filter { it.timestamp >= cutoffSec }
        }
    }

    Card(
        modifier = modifier.fillMaxWidth(),
        colors = androidx.compose.material3.CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.background
        )
    ) {
        Column(Modifier.padding(16.dp)) {
            // Price header
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
                            SimpleDateFormat("MMM d, yyyy", Locale.US)
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
                    val changeColor = if (isUp) Color(0xFF10B981) else Color(0xFFEF4444)
                    val changePercent = if (firstPrice > 0) ((displayPrice - firstPrice) / firstPrice) * 100 else 0.0

                    Column(horizontalAlignment = Alignment.End) {
                        Text(
                            chartPeriod.label,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            String.format("%+.2f%%", changePercent),
                            color = changeColor,
                            fontWeight = FontWeight.SemiBold,
                            fontSize = 14.sp
                        )
                    }
                }
            }

            Spacer(Modifier.height(8.dp))

            // Period selector pills — scrollable
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
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
                            fontSize = 11.sp,
                            fontWeight = FontWeight.Bold,
                            color = if (selected) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 10.dp, vertical = 5.dp),
                            textAlign = TextAlign.Center
                        )
                    }
                }
            }

            Spacer(Modifier.height(12.dp))

            if (priceHistory.size >= 2) {
                val prices = priceHistory.map { it.price }
                val minPrice = prices.min() * 0.98
                val maxPrice = prices.max() * 1.02
                val priceRange = maxPrice - minPrice
                val firstPrice = prices.first()
                val displayPrice = selectedPoint?.price ?: currentPrice
                val isUp = displayPrice >= firstPrice
                val lineColor = if (isUp) Color(0xFF10B981) else Color(0xFFEF4444)

                val selectedIndex = selectedPoint?.let { sp ->
                    priceHistory.indexOfFirst { it.id == sp.id }.takeIf { it >= 0 }
                }

                // Chart with Y-axis labels
                Row(Modifier.fillMaxWidth()) {
                    // Chart canvas
                    Canvas(
                        modifier = Modifier
                            .weight(1f)
                            .height(160.dp)
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

                        if (priceRange < 0.01) {
                            drawLine(color = lineColor, start = Offset(0f, h / 2), end = Offset(w, h / 2), strokeWidth = 2f)
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
                                pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 8f))
                            )
                        }

                        // Line path
                        val linePath = Path()
                        priceHistory.forEachIndexed { i, record ->
                            val px = (i.toFloat() / (priceHistory.size - 1)) * w
                            val py = h - ((record.price - minPrice) / priceRange).toFloat() * h
                            if (i == 0) linePath.moveTo(px, py) else linePath.lineTo(px, py)
                        }

                        // Area fill
                        val areaPath = Path().apply {
                            addPath(linePath)
                            lineTo(w, h)
                            lineTo(0f, h)
                            close()
                        }
                        drawPath(
                            path = areaPath,
                            brush = Brush.verticalGradient(
                                colors = listOf(lineColor.copy(alpha = 0.15f), lineColor.copy(alpha = 0.02f))
                            ),
                            style = Fill
                        )

                        // Line
                        drawPath(path = linePath, color = lineColor, style = Stroke(width = if (selectedIndex != null) 1.5f else 2f))

                        // Selected indicator
                        if (selectedIndex != null) {
                            val sx = (selectedIndex.toFloat() / (priceHistory.size - 1)) * w
                            val record = priceHistory[selectedIndex]
                            val sy = h - ((record.price - minPrice) / priceRange).toFloat() * h
                            drawLine(Color.Gray.copy(alpha = 0.5f), Offset(sx, 0f), Offset(sx, h), 1f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 6f)))
                            drawCircle(lineColor, 5f, Offset(sx, sy))
                            drawCircle(Color.White, 3f, Offset(sx, sy))
                        }
                    }

                    // Y-axis labels
                    Column(
                        modifier = Modifier.height(160.dp).padding(start = 4.dp),
                        verticalArrangement = Arrangement.SpaceBetween
                    ) {
                        for (i in 0..3) {
                            val price = maxPrice - (maxPrice - minPrice) * i / 3
                            Text(
                                formatYAxis(price),
                                fontSize = 9.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                }

                Spacer(Modifier.height(4.dp))

                // X-axis time labels
                val xFmt = when {
                    chartPeriod == ChartPeriod.DAY_1 -> SimpleDateFormat("ha", Locale.US)
                    chartPeriod.effectiveDays() <= 90 -> SimpleDateFormat("MMM d", Locale.US)
                    chartPeriod.effectiveDays() <= 365 -> SimpleDateFormat("MMM", Locale.US)
                    else -> SimpleDateFormat("yyyy", Locale.US)
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    val step = maxOf(priceHistory.size / 4, 1)
                    for (i in listOf(0, step, step * 2, step * 3, priceHistory.size - 1).distinct()) {
                        if (i < priceHistory.size) {
                            Text(
                                xFmt.format(Date(priceHistory[i].timestamp * 1000)),
                                fontSize = 9.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                }
            } else {
                Box(
                    modifier = Modifier.fillMaxWidth().height(160.dp),
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

private fun formatYAxis(price: Double): String {
    return if (price >= 1000) "$${(price / 1000).toInt()}K" else "$${price.toInt()}"
}
