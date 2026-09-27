package com.stablechannels.app.ui.home

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.models.PriceRecord
import com.stablechannels.app.util.usdFormatted
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@Composable
fun PriceChartHeader(
    isExpanded: Boolean,
    chevronRotation: Float,
    selectedPoint: PriceRecord?,
    chartPeriod: ChartPeriod,
    livePriceText: String,
    onToggleExpanded: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .clickable { onToggleExpanded() }
                .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Bitcoin circle badge
        Box(
            modifier = Modifier.size(24.dp).background(Color(0xFFFF9500), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Default.CurrencyBitcoin,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(16.dp),
            )
        }

        Spacer(modifier = Modifier.width(10.dp))

        // Title and optional scrubber timestamp
        Column {
            Text(
                text = "BTC Price",
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.Medium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (selectedPoint != null) {
                val dateFmt =
                    if (chartPeriod == ChartPeriod.DAY_1) {
                        SimpleDateFormat("h:mm a", Locale.US)
                    } else {
                        SimpleDateFormat("MMM d, yyyy", Locale.US)
                    }
                Text(
                    dateFmt.format(Date(selectedPoint.timestamp * 1000)),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Spacer(modifier = Modifier.weight(1f))

        // Price display (pushed to the far right)
        val displayPrice = selectedPoint?.price?.usdFormatted() ?: livePriceText
        Text(
            text = displayPrice,
            fontSize = 17.sp,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.onSurface,
        )

        Spacer(modifier = Modifier.width(10.dp))

        // Rotating chevron
        Icon(
            imageVector = Icons.Default.ChevronRight,
            contentDescription = if (isExpanded) "Collapse price chart" else "Expand price chart",
            modifier = Modifier.size(18.dp).rotate(chevronRotation),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
