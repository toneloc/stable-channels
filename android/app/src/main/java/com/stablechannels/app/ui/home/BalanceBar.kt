package com.stablechannels.app.ui.home

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.usdFormatted
import kotlin.math.max

@Composable
fun BalanceBar(
    stableUSD: Double,
    nativeSats: Long,
    totalSats: Long,
    btcPrice: Double
) {
    val nativeUSD = (nativeSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val totalUSD = stableUSD + nativeUSD
    if (totalUSD <= 0) return

    val stableFraction = (stableUSD / totalUSD).coerceIn(0.0, 1.0).toFloat()
    val nativeFraction = (nativeUSD / totalUSD).coerceIn(0.0, 1.0).toFloat()

    val stableColor = Color(0xFF10B981)
    val nativeColor = Color(0xFFF59E0B)

    Column(modifier = Modifier.fillMaxWidth()) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(12.dp)
                .clip(RoundedCornerShape(6.dp))
        ) {
            Row(Modifier.fillMaxSize()) {
                if (stableFraction > 0.01f) {
                    Box(
                        Modifier
                            .weight(max(stableFraction, 0.03f))
                            .fillMaxHeight()
                            .background(Brush.horizontalGradient(listOf(stableColor, stableColor.copy(alpha = 0.7f))))
                    )
                }
                if (nativeFraction > 0.01f) {
                    Box(
                        Modifier
                            .weight(max(nativeFraction, 0.03f))
                            .fillMaxHeight()
                            .background(Brush.horizontalGradient(listOf(nativeColor.copy(alpha = 0.7f), nativeColor)))
                    )
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
