package com.stablechannels.app.ui.home

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PriceRecord
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.ui.components.CurvePattern
import com.stablechannels.app.ui.components.CurveProgressIndicator
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

@Composable
fun PriceChart(
    appState: AppState,
    databaseService: DatabaseService?,
    currentPrice: Double,
    modifier: Modifier = Modifier
) {
    var chartPeriod by remember { mutableStateOf(ChartPeriod.ALL) }
    var priceHistory by remember { mutableStateOf(emptyList<PriceRecord>()) }
    var selectedPoint by remember { mutableStateOf<PriceRecord?>(null) }

    var allDailyPrices by remember { mutableStateOf(appState.cachedChartDaily) }
    var hourlyPrices by remember { mutableStateOf(appState.cachedChartHourly) }
    var dataLoaded by remember { mutableStateOf(appState.chartDataLoaded) }

    val chartUpdateTrigger by appState.chartUpdateTrigger.collectAsState()

    // Load/reload data when triggered by startup or backfill completion
    LaunchedEffect(chartUpdateTrigger) {
        withContext(Dispatchers.IO) {
            val hourly = databaseService?.getPriceHistory(24 * 30) ?: emptyList()
            val dailyPrices = databaseService?.getDailyPrices(99999) ?: emptyList()
            val fmt = SimpleDateFormat("yyyy-MM-dd", Locale.US).apply {
                timeZone = TimeZone.getTimeZone("UTC")
            }
            val daily = dailyPrices.mapNotNull { d ->
                val date = try { fmt.parse(d.date) } catch (_: Exception) { null } ?: return@mapNotNull null
                val ts = date.time / 1000
                PriceRecord(id = ts, price = d.close, source = "daily", timestamp = ts)
            }.sortedBy { it.timestamp }

            hourlyPrices = hourly
            allDailyPrices = daily
            appState.cachedChartHourly = hourly
            appState.cachedChartDaily = daily
            if (hourly.isNotEmpty() || daily.isNotEmpty()) {
                appState.chartDataLoaded = true
            }
        }
        if (hourlyPrices.isNotEmpty() || allDailyPrices.isNotEmpty()) {
            dataLoaded = true
        }
    }

    // Filter when period changes or data updates
    LaunchedEffect(chartPeriod, dataLoaded, hourlyPrices, allDailyPrices) {
        if (!dataLoaded) return@LaunchedEffect
        selectedPoint = null
        val cutoffMs = System.currentTimeMillis() - chartPeriod.effectiveDays().toLong() * 86400 * 1000
        val cutoffSec = cutoffMs / 1000

        val raw = if (chartPeriod.usesHourly) {
            val startIdx = PriceChartAlgorithms.lowerBound(hourlyPrices, cutoffSec)
            val hourlySlice = hourlyPrices.subList(startIdx, hourlyPrices.size)
            if (hourlySlice.size >= 2) {
                hourlySlice
            } else {
                // Fallback to daily if hourly is sparse or still backfilling
                val dailyStartIdx = PriceChartAlgorithms.lowerBound(allDailyPrices, cutoffSec)
                val dailySlice = allDailyPrices.subList(dailyStartIdx, allDailyPrices.size)
                if (dailySlice.size >= 2) dailySlice else hourlySlice
            }
        } else {
            val startIdx = PriceChartAlgorithms.lowerBound(allDailyPrices, cutoffSec)
            val dailySlice = allDailyPrices.subList(startIdx, allDailyPrices.size)
            if (dailySlice.size >= 2) {
                dailySlice
            } else {
                // Fallback to hourly if daily is sparse or still backfilling
                val hourlyStartIdx = PriceChartAlgorithms.lowerBound(hourlyPrices, cutoffSec)
                val hourlySlice = hourlyPrices.subList(hourlyStartIdx, hourlyPrices.size)
                if (hourlySlice.size >= 2) hourlySlice else dailySlice
            }
        }
        priceHistory = PriceChartAlgorithms.lttbDownsample(raw, 200)
    }

    val livePriceText by remember(currentPrice) {
        derivedStateOf { currentPrice.usdFormatted() }
    }

    Card(
        modifier = modifier.fillMaxWidth(),
        colors = androidx.compose.material3.CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.background
        )
    ) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
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
                            livePriceText,
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
                            String.format(Locale.US, "%+.2f%%", changePercent),
                            color = changeColor,
                            fontWeight = FontWeight.SemiBold,
                            fontSize = 14.sp
                        )
                    }
                }
            }

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
                        shape = RoundedCornerShape(20.dp),
                        color = if (selected) Color(0xFF3B82F6) else MaterialTheme.colorScheme.surfaceVariant,
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

            Spacer(Modifier.height(4.dp))

            if (priceHistory.size >= 2) {
                // Single-pass min/max calculation
                val (minPrice, maxPrice) = remember(priceHistory) {
                    PriceChartAlgorithms.minMaxPrices(priceHistory)
                }
                val priceRange = maxPrice - minPrice
                val firstPrice = priceHistory.first().price
                val displayPrice = selectedPoint?.price ?: currentPrice
                val isUp = displayPrice >= firstPrice
                val lineColor = if (isUp) Color(0xFF10B981) else Color(0xFFEF4444)

                val selectedIndex = selectedPoint?.let { sp ->
                    priceHistory.indexOfFirst { it.id == sp.id }.takeIf { it >= 0 }
                }

                // Chart with Y-axis labels
                Row(Modifier.fillMaxWidth()) {
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
                            brush = Brush.verticalGradient(
                                colors = listOf(lineColor.copy(alpha = 0.15f), lineColor.copy(alpha = 0.02f))
                            ),
                            style = Fill
                        )

                        // Line
                        drawPath(
                            path = linePath,
                            color = lineColor,
                            style = Stroke(width = if (selectedIndex != null) 1.5f else 2f)
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
                                pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 6f))
                            )
                            drawCircle(lineColor, 5f, Offset(sx, sy))
                            drawCircle(Color.White, 3f, Offset(sx, sy))
                        }
                    }

                    // Y-axis labels
                    Column(
                        modifier = Modifier.height(160.dp).padding(start = 4.dp),
                        verticalArrangement = Arrangement.SpaceBetween,
                        horizontalAlignment = Alignment.End
                    ) {
                        for (i in 4 downTo 0) {
                            val price = minPrice + (priceRange * i / 4)
                            Text(
                                PriceChartAlgorithms.formatYAxis(price),
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
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        CurveProgressIndicator(
                            size = 68.dp,
                            pattern = CurvePattern.SPIRAL_SEARCH,
                            primaryColor = Color(0xFF38BDF8)
                        )
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
}

@Preview(showBackground = true)
@Composable
private fun PriceChartCollectingDataPreview() {
    MaterialTheme {
        Card(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            colors = androidx.compose.material3.CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.background
            )
        ) {
            Box(
                modifier = Modifier.fillMaxWidth().height(160.dp),
                contentAlignment = Alignment.Center
            ) {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    CurveProgressIndicator(
                        size = 68.dp,
                        pattern = CurvePattern.SPIRAL_SEARCH,
                        primaryColor = Color(0xFF38BDF8)
                    )
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
