package com.stablechannels.app.ui.settings

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.components.NeutralOutlinedButton
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.ui.components.OfflineBadge
import com.stablechannels.app.util.OfflineMessages
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.openInAppBrowser
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@Composable
fun ChannelView(appState: AppState) {
    val sc by appState.stableChannel.collectAsState()
    val scope = rememberCoroutineScope()
    var showCloseConfirm by remember { mutableStateOf(false) }

    // Observe the closing flag and channel readiness as Compose state. This screen used to read
    // the plain appState.isChannelClosing getter and derive readiness from nodeService.channels,
    // neither of which is observable — so when a close confirmed while this screen was open, the
    // flag cleared but nothing recomposed and "Closing channel..." spun until the screen was left
    // and reopened (e2e flow 09). Both flows flip at close; reading them re-reads the list.
    val isClosing by appState.isChannelClosingFlow.collectAsState()
    val hasReadyChannel by appState.hasReadyChannel.collectAsState()
    val channels = appState.nodeService.channels
    val isOnline by appState.isOnline.collectAsState()
    val lightningSats by appState.lightningBalanceSats.collectAsState()
    val hasCachedChannel = hasReadyChannel || sc.userChannelId.isNotEmpty()

    Column(modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        if (channels.isNotEmpty() && !isClosing) {
            val ch = channels.first()

            ChannelStatusRow(isReady = ch.isChannelReady, isOnline = isOnline)

            Spacer(Modifier.height(20.dp))

            // Capacity
            ChannelDetailRow("Capacity", "${ch.channelValueSats.toLong().btcSpacedFormatted()} BTC")
            Spacer(Modifier.height(16.dp))

            // Outbound
            ChannelDetailRow(
                "Outbound",
                "${(ch.outboundCapacityMsat.toLong() / 1000).btcSpacedFormatted()} BTC",
            )
            Spacer(Modifier.height(16.dp))

            // Inbound
            ChannelDetailRow(
                "Inbound",
                "${(ch.inboundCapacityMsat.toLong() / 1000).btcSpacedFormatted()} BTC",
            )

            FundingTxCard(appState.fundingTxid)

            if (hasReadyChannel) {
                Spacer(Modifier.height(32.dp))
                CloseChannelButton(enabled = isOnline, onClick = { showCloseConfirm = true })
            }
        } else if (isClosing) {
            // Channel is closing — show status
            Spacer(Modifier.height(32.dp))
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(48.dp),
                    color = Color(0xFFF59E0B),
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    text = "Closing channel...",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Medium,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "Funds will be swept to your onchain wallet",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        } else if (hasCachedChannel) {
            ChannelStatusRow(isReady = hasReadyChannel, isOnline = isOnline)
            Spacer(Modifier.height(20.dp))
            ChannelDetailRow("Capacity", "${lightningSats.btcSpacedFormatted()} BTC")
            FundingTxCard(appState.fundingTxid)
            if (!isOnline) {
                Spacer(Modifier.height(32.dp))
                CloseChannelButton(enabled = false, onClick = {})
            }
        } else {
            Text(
                text = "No channel open yet",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = "Receive bitcoin over Lightning to open your first channel.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }

    if (showCloseConfirm) {
        AlertDialog(
            onDismissRequest = { showCloseConfirm = false },
            containerColor = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            title = { Text("Close channel") },
            text = {
                Text(
                    "This will cooperatively close the channel and return your funds to your onchain wallet after confirmation."
                )
            },
            confirmButton = {
                NeutralTextButton(
                    onClick = {
                        showCloseConfirm = false
                        appState.isChannelClosing = true
                        appState.setStatus("Closing channel...")
                        appState.prepareChannelCloseTracking(sc.userChannelId)
                        scope.launch(Dispatchers.IO) {
                            try {
                                appState.nodeService.closeChannel(sc.userChannelId, sc.counterparty)
                                appState.refreshBalances()
                            } catch (e: Exception) {
                                appState.setStatus("Close failed: ${e.message}")
                                appState.isChannelClosing = false
                            }
                        }
                    }
                ) {
                    Text("Close channel", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                NeutralTextButton(onClick = { showCloseConfirm = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun ChannelDetailRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = label, style = MaterialTheme.typography.bodyLarge)
        Text(
            text = value,
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.Medium,
        )
    }
}

@Composable
private fun ChannelStatusRow(isReady: Boolean, isOnline: Boolean) {
    val (label, color) =
        when {
            !isOnline -> "Offline" to Color(0xFFF59E0B)
            isReady -> "Ready" to Color(0xFF10B981)
            else -> "Pending" to Color(0xFFF59E0B)
        }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Status", style = MaterialTheme.typography.bodyLarge)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Surface(
                shape = MaterialTheme.shapes.small,
                color = color,
                modifier = Modifier.size(8.dp),
            ) {}
            Text(
                text = label,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
                color = color,
            )
        }
    }
}

@Composable
private fun FundingTxCard(fundingTxid: String?) {
    val context = LocalContext.current
    fundingTxid?.let { txid ->
        if (txid.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            Surface(
                shape = MaterialTheme.shapes.medium,
                tonalElevation = 1.dp,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(
                        text = "Funding Transaction",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "${txid.take(8)}...${txid.takeLast(8)}",
                        style = MaterialTheme.typography.bodyMedium,
                        fontFamily = FontFamily.Monospace,
                    )
                    Spacer(Modifier.height(8.dp))
                    NeutralTextButton(
                        onClick = {
                            context.openInAppBrowser(
                                "https://mempool.space/tx/${txid.substringBefore(":")}"
                            )
                        },
                        contentPadding = PaddingValues(0.dp),
                    ) {
                        Text("View on explorer ↗")
                    }
                }
            }
        }
    }
}

@Composable
private fun CloseChannelButton(enabled: Boolean, onClick: () -> Unit) {
    val red = Color(0xFFEF4444)
    if (!enabled) {
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
            OfflineBadge(info = OfflineMessages.CLOSE_CHANNEL, tooltipMargin = 16.dp)
        }
        Spacer(Modifier.height(12.dp))
    }
    NeutralOutlinedButton(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.fillMaxWidth(),
        colors = ButtonDefaults.outlinedButtonColors(contentColor = red),
        border = BorderStroke(1.dp, if (enabled) red else red.copy(alpha = 0.38f)),
    ) {
        Text("Close channel")
    }
}
