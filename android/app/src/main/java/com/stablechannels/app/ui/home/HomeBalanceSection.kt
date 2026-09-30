package com.stablechannels.app.ui.home

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted

@Composable
fun HomeBalanceSection(
    totalSats: Long,
    totalUSD: Double,
    btcPrice: Double,
    showBTC: Boolean,
    isFlashing: Boolean,
    onToggleShowBTC: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = modifier.clickable(onClick = onToggleShowBTC).paymentFlash(isFlashing),
    ) {
        Text(
            text = "Total Balance",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(4.dp))
        if (showBTC) {
            RollingDigitText(
                text = totalSats.btcSpacedFormatted() + " BTC",
                style =
                    MaterialTheme.typography.headlineLarge.copy(
                        fontSize = 32.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = FontFamily.Monospace,
                    ),
            )
        } else {
            if (btcPrice > 0) {
                RollingDigitText(
                    text = totalUSD.usdFormatted(),
                    style =
                        MaterialTheme.typography.headlineLarge.copy(
                            fontSize = 36.sp,
                            fontWeight = FontWeight.Bold,
                        ),
                )
            } else if (totalSats > 0) {
                Text(
                    text = "Fetching price...",
                    fontSize = 24.sp,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Text(
                    text = "$0.00",
                    fontSize = 36.sp,
                    fontWeight = FontWeight.Bold,
                )
            }
        }
        Text(
            text =
                if (showBTC) {
                    if (btcPrice > 0) totalUSD.usdFormatted() else "—"
                } else totalSats.btcSpacedFormatted() + " BTC",
            fontSize = 14.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
