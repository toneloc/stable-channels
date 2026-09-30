package com.stablechannels.app.ui.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.stablechannels.app.ui.components.CurvePattern
import com.stablechannels.app.ui.components.CurveProgressIndicator

@Composable
fun PriceChartLoadingView(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier.fillMaxWidth().height(160.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            CurveProgressIndicator(
                size = 68.dp,
                pattern = CurvePattern.SPIRAL_SEARCH,
                primaryColor = Color(0xFF38BDF8),
            )
            Text(
                "Collecting price data...",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
