package com.stablechannels.app.ui.settings

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
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
import com.stablechannels.app.util.satsFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@Composable
fun ChannelView(appState: AppState) {
    val sc by appState.stableChannel.collectAsState()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var showCloseConfirm by remember { mutableStateOf(false) }

    val channels = appState.nodeService.channels
    val hasReadyChannel = channels.any { it.isChannelReady }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        if (channels.isNotEmpty() && !appState.isChannelClosing) {
            val ch = channels.first()

            // Status with colored dot
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("Status", style = MaterialTheme.typography.bodyLarge)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Surface(
                        shape = MaterialTheme.shapes.small,
                        color = if (ch.isChannelReady) Color(0xFF10B981) else Color(0xFFF59E0B),
                        modifier = Modifier.size(8.dp)
                    ) {}
                    Text(
                        text = if (ch.isChannelReady) "Ready" else "Pending",
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium,
                        color = if (ch.isChannelReady) Color(0xFF10B981) else Color(0xFFF59E0B)
                    )
                }
            }

            Spacer(Modifier.height(20.dp))

            // Capacity
            ChannelDetailRow("Capacity", ch.channelValueSats.toLong().satsFormatted())
            Spacer(Modifier.height(16.dp))

            // Outbound
            ChannelDetailRow("Outbound", (ch.outboundCapacityMsat.toLong() / 1000).satsFormatted())
            Spacer(Modifier.height(16.dp))

            // Inbound
            ChannelDetailRow("Inbound", (ch.inboundCapacityMsat.toLong() / 1000).satsFormatted())

            // Funding Tx
            appState.fundingTxid?.let { txid ->
                if (txid.isNotEmpty()) {
                    Spacer(Modifier.height(20.dp))
                    Surface(
                        shape = MaterialTheme.shapes.medium,
                        tonalElevation = 1.dp,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Column(modifier = Modifier.padding(16.dp)) {
                            Text(
                                text = "Funding Transaction",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Spacer(Modifier.height(4.dp))
                            Text(
                                text = "${txid.take(8)}...${txid.takeLast(8)}",
                                style = MaterialTheme.typography.bodyMedium,
                                fontFamily = FontFamily.Monospace
                            )
                            Spacer(Modifier.height(8.dp))
                            TextButton(
                                onClick = {
                                    val intent = Intent(Intent.ACTION_VIEW, Uri.parse("https://mempool.space/tx/$txid"))
                                    context.startActivity(intent)
                                },
                                contentPadding = PaddingValues(0.dp)
                            ) {
                                Text("View on explorer ↗", color = Color(0xFF3B82F6))
                            }
                        }
                    }
                }
            }

            if (hasReadyChannel) {
                Spacer(Modifier.height(32.dp))
                OutlinedButton(
                    onClick = { showCloseConfirm = true },
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.outlinedButtonColors(
                        contentColor = Color(0xFFEF4444)
                    ),
                    border = androidx.compose.foundation.BorderStroke(1.dp, Color(0xFFEF4444))
                ) {
                    Text("Close Channel")
                }
            }
        } else if (appState.isChannelClosing) {
            // Channel is closing — show status
            Spacer(Modifier.height(32.dp))
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(48.dp),
                    color = Color(0xFFF59E0B)
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    text = "Closing channel...",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Medium
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "Funds will be swept to your on-chain wallet",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        } else {
            Text(
                text = "No channel open yet",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = "Receive bitcoin over Lightning to open your first channel.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }

    if (showCloseConfirm) {
        AlertDialog(
            onDismissRequest = { showCloseConfirm = false },
            title = { Text("Close Channel?") },
            text = { Text("This will cooperatively close the channel and sweep funds on-chain.") },
            confirmButton = {
                TextButton(onClick = {
                    showCloseConfirm = false
                    appState.isChannelClosing = true
                    appState.setStatus("Closing channel...")
                    scope.launch(Dispatchers.IO) {
                        try {
                            appState.nodeService.closeChannel(sc.userChannelId, sc.counterparty)
                            appState.refreshBalances()
                        } catch (e: Exception) {
                            appState.setStatus("Close failed: ${e.message}")
                            appState.isChannelClosing = false
                        }
                    }
                }) { Text("Close", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { showCloseConfirm = false }) { Text("Cancel") }
            }
        )
    }
}

@Composable
private fun ChannelDetailRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(text = label, style = MaterialTheme.typography.bodyLarge)
        Text(text = value, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
    }
}
