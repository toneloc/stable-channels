package com.stablechannels.app.ui.home

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.ui.theme.LocalDarkTheme

@Composable
fun PriceChartPeriodSelector(
    selectedPeriod: ChartPeriod,
    onPeriodSelected: (ChartPeriod) -> Unit,
    modifier: Modifier = Modifier,
) {
    val isDark = LocalDarkTheme.current
    val haptic = LocalHapticFeedback.current

    Row(
        modifier = modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        ChartPeriod.entries.forEach { period ->
            val selected = selectedPeriod == period
            val pillBg =
                if (selected) Color(0xFF3B82F6)
                else if (isDark) Color(0xFF2C2C2E) else Color(0xFFE5E5EA)
            val pillText =
                if (selected) Color.White else if (isDark) Color.White else Color(0xFF8E8E93)
            Box(
                modifier =
                    Modifier.clip(RoundedCornerShape(8.dp))
                        .background(pillBg)
                        .clickable {
                            if (selectedPeriod != period) {
                                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                onPeriodSelected(period)
                            }
                        }
                        .padding(horizontal = 10.dp, vertical = 5.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = period.label,
                    fontSize = 11.sp,
                    fontWeight = if (selected) FontWeight.Bold else FontWeight.Medium,
                    color = pillText,
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}
