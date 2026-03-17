package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import java.io.File
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PendingSplice
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.StabilityService
import com.stablechannels.app.ui.transfer.OnChainSendScreen
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import withContext
import org.lightningdevkit.ldknode.Network

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(appState: AppState, modifier: Modifier = Modifier) {
    val sc by appState.stableChannel.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val onchainSats by appState.onchainBalanceSats.collectAsState()
    val clipboardManager = LocalClipboardManager.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var showNodeId by remember { mutableStateOf(false) }
    var showCloseConfirm by remember { mutableStateOf(false) }
    var showOnchainSend by remember { mutableStateOf(false) }
    var showSeedWords by remember { mutableStateOf(false) }
    var showRestore by remember { mutableStateOf(false) }
    var restoreMnemonic by remember { mutableStateOf("") }
    var restoreError by remember { mutableStateOf<String?>(null) }

    val channels = appState.nodeService.channels
    val hasReadyChannel = channels.any { it.isChannelReady }
    val stabilityResult = remember(sc, btcPrice) {
        StabilityService.checkStabilityAction(sc, btcPrice)
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp)
    ) {
        Text("Settings", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(16.dp))

        // Node section
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp)) {
                Text("Node", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(8.dp))

                Row(verticalAlignment = Alignment.CenterVertically) {
                    val statusColor = if (appState.nodeService.isRunning) Color(0xFF10B981) else Color.Gray
                    Surface(
                        shape = MaterialTheme.shapes.small,
                        color = statusColor,
                        modifier = Modifier.size(8.dp)
                    ) {}
                    Spacer(Modifier.width(8.dp))
                    Text(if (appState.nodeService.isRunning) "Running" else "Stopped")
                }

                Spacer(Modifier.height(8.dp))
                if (showNodeId && appState.nodeService.nodeId.isNotEmpty()) {
                    val nodeId = appState.nodeService.nodeId
                    Text(
                        text = "${nodeId.take(8)}...${nodeId.takeLast(8)}",
                        fontFamily = FontFamily.Monospace,
                        style = MaterialTheme.typography.bodySmall
                    )
                    Spacer(Modifier.height(4.dp))
                    TextButton(onClick = {
                        clipboardManager.setText(AnnotatedString(nodeId))
                    }) { Text("Copy Node ID") }
                } else {
                    TextButton(onClick = { showNodeId = true }) { Text("Show Node ID") }
                }
            }
        }

        // Channel section
        if (channels.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Channel", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(8.dp))

                    val ch = channels.first()
                    DetailRow("Capacity", ch.channelValueSats.toLong().satsFormatted())
                    DetailRow("Status", if (ch.isChannelReady) "Ready" else "Pending")
                    DetailRow("Outbound", (ch.outboundCapacityMsat.toLong() / 1000).satsFormatted())
                    DetailRow("Inbound", (ch.inboundCapacityMsat.toLong() / 1000).satsFormatted())
                }
            }
        }

        // Stable Position
        if (sc.expectedUSD.amount > 0) {
            Spacer(Modifier.height(12.dp))
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Stable Position", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(8.dp))

                    DetailRow("Expected USD", sc.expectedUSD.formatted)
                    DetailRow("Backing Sats", sc.backingSats.satsFormatted())
                    DetailRow("Native BTC", sc.nativeChannelBTC.formatted)
                    DetailRow("Stability", stabilityResult.action.value)
                    if (stabilityResult.percentFromPar > 0) {
                        DetailRow("% From Par", String.format("%.4f%%", stabilityResult.percentFromPar))
                    }

                    Spacer(Modifier.height(4.dp))
                    val cpk = sc.counterparty
                    Text(
                        "Counterparty: ${cpk.take(8)}...${cpk.takeLast(8)}",
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace
                    )
                }
            }
        }

        // Close channel
        if (hasReadyChannel) {
            Spacer(Modifier.height(12.dp))
            OutlinedButton(
                onClick = { showCloseConfirm = true },
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error)
            ) { Text("Close Channel") }
        }

        // On-chain
        Spacer(Modifier.height(16.dp))
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp)) {
                Text("On-Chain", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(8.dp))
                DetailRow("Balance", onchainSats.satsFormatted())

                TextButton(onClick = { showOnchainSend = true }) { Text("Send On-Chain") }
            }
        }

        // Backup
        Spacer(Modifier.height(16.dp))
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp)) {
                Text("Backup", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(8.dp))
                Button(
                    onClick = { showSeedWords = !showSeedWords },
                    modifier = Modifier.fillMaxWidth()
                ) { Text(if (showSeedWords) "Hide Seed Words" else "Backup Seed Words") }
                if (showSeedWords) {
                    val words = appState.nodeService.savedMnemonic
                    if (!words.isNullOrEmpty()) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            "Write these words down on paper and store them in a safe place. Never share them. Anyone with these words can access your funds.",
                            style = MaterialTheme.typography.bodySmall,
                            color = Color(0xFFD97706)
                        )
                        Spacer(Modifier.height(8.dp))
                        words.split(" ").forEachIndexed { index, word ->
                            Text(
                                "${index + 1}. $word",
                                fontFamily = FontFamily.Monospace,
                                style = MaterialTheme.typography.bodyMedium
                            )
                        }
                        Spacer(Modifier.height(8.dp))
                        var copied by remember { mutableStateOf(false) }
                        OutlinedButton(
                            onClick = {
                                clipboardManager.setText(AnnotatedString(words))
                                copied = true
                            },
                            modifier = Modifier.fillMaxWidth()
                        ) { Text(if (copied) "Copied" else "Copy Seed Words") }
                    } else {
                        Spacer(Modifier.height(8.dp))
                        Text("Seed phrase not available for this wallet.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                Spacer(Modifier.height(8.dp))
                OutlinedButton(
                    onClick = { showRestore = true },
                    modifier = Modifier.fillMaxWidth()
                ) { Text("Restore from Seed") }
            }
        }

        // About
        Spacer(Modifier.height(16.dp))
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp)) {
                Text("About", style = MaterialTheme.typography.titleMedium)
                DetailRow("Version", "1.0")
                DetailRow("Network", "bitcoin")
            }
        }

        Spacer(Modifier.height(32.dp))
    }

    // Close channel confirmation
    if (showCloseConfirm) {
        AlertDialog(
            onDismissRequest = { showCloseConfirm = false },
            title = { Text("Close Channel?") },
            text = { Text("This will close your Lightning channel and return funds on-chain. Are you sure?") },
            confirmButton = {
                TextButton(onClick = {
                    showCloseConfirm = false
                    scope.launch(Dispatchers.IO) {
                        try {
                            appState.nodeService.closeChannel(sc.userChannelId, sc.counterparty)
                        } catch (_: Exception) {}
                    }
                }) { Text("Close", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { showCloseConfirm = false }) { Text("Cancel") }
            }
        )
    }

    // Restore from seed dialog
    if (showRestore) {
        AlertDialog(
            onDismissRequest = {
                showRestore = false
                restoreMnemonic = ""
                restoreError = null
            },
            title = { Text("Restore from Seed") },
            text = {
                Column {
                    Text(
                        "Enter your 12 or 24-word seed phrase to restore a wallet.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = restoreMnemonic,
                        onValueChange = { restoreMnemonic = it },
                        label = { Text("word1 word2 word3 ...") },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3
                    )
                    if (restoreError != null) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            restoreError!!,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val input = restoreMnemonic.trim()
                        val wordCount = input.split("\\s+".toRegex()).size
                        if (wordCount != 12 && wordCount != 24) {
                            restoreError = "Seed phrase must be 12 or 24 words"
                            return@TextButton
                        }
                        scope.launch(Dispatchers.IO) {
                            try {
                                appState.nodeService.stop()
                                appState.nodeService.start(Network.BITCOIN, Constants.PRIMARY_CHAIN_URL, input)
                                withContext(Dispatchers.Main) {
                                    showRestore = false
                                    restoreMnemonic = ""
                                    restoreError = null
                                    appState.refreshBalances()
                                }
                            } catch (e: Exception) {
                                withContext(Dispatchers.Main) {
                                    restoreError = e.message ?: "Restore failed"
                                }
                            }
                        }
                    },
                    enabled = restoreMnemonic.trim().isNotEmpty()
                ) { Text("Restore") }
            },
            dismissButton = {
                TextButton(onClick = {
                    showRestore = false
                    restoreMnemonic = ""
                    restoreError = null
                }) { Text("Cancel") }
            }
        )
    }

    // On-chain send sheet
    if (showOnchainSend) {
        ModalBottomSheet(onDismissRequest = { showOnchainSend = false }) {
            OnChainSendScreen(appState) { showOnchainSend = false }
        }
    }
}

@Composable
private fun DetailRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}
