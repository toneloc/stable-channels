package com.stablechannels.app.ui.home

import android.content.Context
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PriceRecord
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.ui.theme.LocalDarkTheme
import com.stablechannels.app.util.usdFormatted

@Composable
fun PriceChart(
    appState: AppState,
    databaseService: DatabaseService?,
    currentPrice: Double,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val haptic = LocalHapticFeedback.current
    val prefs =
        remember(context) { context.getSharedPreferences("app_prefs", Context.MODE_PRIVATE) }
    var isExpanded by remember {
        mutableStateOf(prefs.getBoolean("is_price_chart_expanded", true))
    }
    val chevronRotation by
        animateFloatAsState(
            targetValue = if (isExpanded) 90f else 0f,
            animationSpec = spring(dampingRatio = 0.82f, stiffness = Spring.StiffnessMediumLow),
            label = "price_chart_chevron_rotation",
        )

    var chartPeriod by remember { mutableStateOf(ChartPeriod.ALL) }
    var priceHistory by remember { mutableStateOf(emptyList<PriceRecord>()) }
    var selectedPoint by remember { mutableStateOf<PriceRecord?>(null) }

    var allDailyPrices by remember { mutableStateOf(appState.cachedChartDaily) }
    var hourlyPrices by remember { mutableStateOf(appState.cachedChartHourly) }
    var dataLoaded by remember { mutableStateOf(appState.chartDataLoaded) }

    val chartUpdateTrigger by appState.chartUpdateTrigger.collectAsState()

    // Load/reload data when triggered by startup or backfill completion
    LaunchedEffect(chartUpdateTrigger) {
        val (hourly, daily) = PriceChartDataLoader.loadPriceData(databaseService, appState)
        hourlyPrices = hourly
        allDailyPrices = daily
        if (hourly.isNotEmpty() || daily.isNotEmpty()) {
            dataLoaded = true
            appState.chartDataLoaded = true
        }
    }

    // Filter when period changes or data updates
    LaunchedEffect(chartPeriod, dataLoaded, hourlyPrices, allDailyPrices) {
        if (!dataLoaded) return@LaunchedEffect
        selectedPoint = null
        val cutoffMs =
            System.currentTimeMillis() - chartPeriod.effectiveDays().toLong() * 86400 * 1000
        val cutoffSec = cutoffMs / 1000
        val raw =
            PriceChartAlgorithms.sliceHistory(
                chartPeriod,
                hourlyPrices,
                allDailyPrices,
                cutoffSec,
            )
        priceHistory = PriceChartAlgorithms.lttbDownsample(raw, 200)
    }

    val livePriceText by
        remember(currentPrice) {
            derivedStateOf {
                if (currentPrice > 0.0) currentPrice.usdFormatted() else "---"
            }
        }

    val isDark = LocalDarkTheme.current
    val surfaceFill = if (isDark) Color(0xFF1E1E1E) else Color(0xFFF5F5F7)
    val borderStroke =
        if (isDark) Color.White.copy(alpha = 0.08f) else Color.Black.copy(alpha = 0.08f)

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        color = surfaceFill,
        border = BorderStroke(1.dp, borderStroke),
        shadowElevation = if (isDark) 0.dp else 1.dp,
    ) {
        Column(modifier = Modifier.fillMaxWidth()) {
            PriceChartHeader(
                isExpanded = isExpanded,
                chevronRotation = chevronRotation,
                selectedPoint = selectedPoint,
                chartPeriod = chartPeriod,
                livePriceText = livePriceText,
                onToggleExpanded = {
                    haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                    val newExpanded = !isExpanded
                    isExpanded = newExpanded
                    prefs.edit().putBoolean("is_price_chart_expanded", newExpanded).apply()
                },
            )

            AnimatedVisibility(
                visible = isExpanded,
                enter =
                    expandVertically(
                        spring(dampingRatio = 0.82f, stiffness = Spring.StiffnessMediumLow)
                    ) + fadeIn(),
                exit =
                    shrinkVertically(
                        spring(dampingRatio = 0.82f, stiffness = Spring.StiffnessMediumLow)
                    ) + fadeOut(),
            ) {
                Column(
                    modifier =
                        Modifier.fillMaxWidth()
                            .padding(start = 14.dp, end = 14.dp, top = 10.dp, bottom = 12.dp)
                ) {
                    PriceChartPeriodSelector(
                        selectedPeriod = chartPeriod,
                        onPeriodSelected = { chartPeriod = it },
                    )

                    Spacer(Modifier.height(12.dp))

                    PriceChartGraph(
                        priceHistory = priceHistory,
                        chartPeriod = chartPeriod,
                        currentPrice = currentPrice,
                        selectedPoint = selectedPoint,
                        onPointSelected = { selectedPoint = it },
                    )
                }
            }
        }
    }
}
