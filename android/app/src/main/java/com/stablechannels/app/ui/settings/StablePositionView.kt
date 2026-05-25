package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.services.StabilityService
import com.stablechannels.app.util.satsFormatted

@Composable
fun StablePositionView(appState: AppState) {
    val sc by appState.stableChannel.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val clipboardManager = LocalClipboardManager.current
    var copiedCounterparty by remember { mutableStateOf(false) }

    val stabilityResult = remember(sc, btcPrice) {
        StabilityService.checkStabilityAction(sc, btcPrice)
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        if (sc.expectedUSD.amount > 0) {
            // Expected USD — prominent
            SettingsDetailRow(
                label = "Expected USD",
                value = sc.expectedUSD.formatted,
                valueColor = Color(0xFF10B981),
                valueBold = true
            )
            Spacer(Modifier.height(16.dp))

            // Backing Sats
            SettingsDetailRow(
                label = "Backing Sats",
                value = sc.backingSats.satsFormatted()
            )
            Spacer(Modifier.height(16.dp))

            // Native BTC
            SettingsDetailRow(
                label = "Native BTC",
                value = sc.nativeChannelBTC.formatted,
                valueColor = Color(0xFFF59E0B)
            )
            Spacer(Modifier.height(16.dp))

            // Stability Status — colored
            val statusColor = when (stabilityResult.action.value) {
                "STABLE" -> Color(0xFF10B981)
                "PAY" -> Color(0xFFF59E0B)
                "CHECK_ONLY" -> Color(0xFF3B82F6)
                "HIGH_RISK_NO_ACTION" -> Color(0xFFEF4444)
                else -> MaterialTheme.colorScheme.onSurface
            }
            SettingsDetailRow(
                label = "Status",
                value = stabilityResult.action.value,
                valueColor = statusColor,
                valueBold = true
            )
            Spacer(Modifier.height(16.dp))

            // Distance from Par — colored
            if (stabilityResult.percentFromPar > 0) {
                val parColor = if (stabilityResult.percentFromPar < 0.1) Color(0xFF10B981) else Color(0xFFF59E0B)
                SettingsDetailRow(
                    label = "Distance from Par",
                    value = String.format("%.2f%%", stabilityResult.percentFromPar),
                    valueColor = parColor
                )
                Spacer(Modifier.height(16.dp))
            }

            // Counterparty — tap to copy
            val cpk = sc.counterparty
            if (cpk.isNotEmpty()) {
                Surface(
                    onClick = {
                        clipboardManager.setText(AnnotatedString(cpk))
                        copiedCounterparty = true
                    },
                    shape = MaterialTheme.shapes.medium,
                    tonalElevation = 1.dp,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Row(
                        modifier = Modifier.padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = "Counterparty",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Spacer(Modifier.height(4.dp))
                            Text(
                                text = "${cpk.take(8)}...${cpk.takeLast(8)}",
                                style = MaterialTheme.typography.bodyMedium,
                                fontFamily = FontFamily.Monospace
                            )
                        }
                        Text(
                            text = if (copiedCounterparty) "Copied ✓" else "Tap to copy",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (copiedCounterparty) Color(0xFF10B981) else MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        } else {
            Text(
                text = "No stable position active",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = "Trade BTC to USD to create a stable position.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun SettingsDetailRow(
    label: String,
    value: String,
    valueColor: Color = Color.Unspecified,
    valueBold: Boolean = false
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurface
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyLarge,
            color = if (valueColor != Color.Unspecified) valueColor else MaterialTheme.colorScheme.onSurface,
            fontWeight = if (valueBold) FontWeight.SemiBold else FontWeight.Normal
        )
    }
}
