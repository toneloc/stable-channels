package com.stablechannels.app.ui.home.balancebar

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted

@Composable
fun BalanceBarLabels(
    stableUSD: Double,
    nativeSats: Long,
    nativeUSD: Double,
    btcPrice: Double,
    showBtcFormat: Boolean,
    isEmpty: Boolean,
    stableColor: Color,
    nativeColor: Color,
    modifier: Modifier = Modifier,
) {
    val stableSats =
        if (btcPrice > 0) (stableUSD / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L

    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        // Left: USD label + amount
        Column {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Icon(
                    Icons.Default.Shield,
                    contentDescription = null,
                    tint = stableColor,
                    modifier = Modifier.size(12.dp),
                )
                Text(
                    "USD",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color = stableColor,
                )
            }
            Text(
                if (showBtcFormat) stableSats.btcSpacedFormatted() + " BTC"
                else stableUSD.usdFormatted(),
                style = MaterialTheme.typography.labelSmall,
                color =
                    if (btcPrice > 0 && !isEmpty) MaterialTheme.colorScheme.onSurface
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // Right: BTC label + amount
        Column(horizontalAlignment = Alignment.End) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    "BTC",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color = nativeColor,
                )
                Icon(
                    Icons.Default.CurrencyBitcoin,
                    contentDescription = null,
                    tint = nativeColor,
                    modifier = Modifier.size(12.dp),
                )
            }
            Text(
                if (showBtcFormat) nativeSats.btcSpacedFormatted() + " BTC"
                else if (btcPrice > 0) nativeUSD.usdFormatted() else "...",
                style = MaterialTheme.typography.labelSmall,
                color =
                    if (btcPrice > 0 && !isEmpty) MaterialTheme.colorScheme.onSurface
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
