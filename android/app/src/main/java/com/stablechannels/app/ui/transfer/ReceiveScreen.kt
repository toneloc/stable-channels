package com.stablechannels.app.ui.transfer

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.home.FundWalletScreen
import com.stablechannels.app.ui.home.generateQRCode
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReceiveScreen(appState: AppState, onDismiss: () -> Unit) {
    var amountSats by remember { mutableStateOf("") }
    var invoice by remember { mutableStateOf<String?>(null) }
    var isGenerating by remember { mutableStateOf(false) }
    var isCopied by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var showOnChain by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val clipboardManager = LocalClipboardManager.current

    val hasChannel = appState.nodeService.channels.any { it.isChannelReady }

    if (showOnChain) {
        FundWalletScreen(appState)
        return
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("Receive", style = MaterialTheme.typography.headlineSmall)
            TextButton(onClick = { showOnChain = true }) { Text("On-Chain") }
        }
        Spacer(Modifier.height(16.dp))

        if (invoice != null) {
            val inv = invoice!!
            val qrBitmap = remember(inv) { generateQRCode(inv.uppercase()) }
            if (qrBitmap != null) {
                Image(
                    bitmap = qrBitmap.asImageBitmap(),
                    contentDescription = "QR Code",
                    modifier = Modifier.size(200.dp)
                )
            }
            Spacer(Modifier.height(12.dp))
            Text(
                text = inv.take(30) + "..." + inv.takeLast(10),
                fontFamily = FontFamily.Monospace,
                style = MaterialTheme.typography.bodySmall,
                textAlign = TextAlign.Center
            )
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    clipboardManager.setText(AnnotatedString(inv))
                    isCopied = true
                }) { Text(if (isCopied) "Copied!" else "Copy") }
                OutlinedButton(onClick = {
                    invoice = null
                    isCopied = false
                }) { Text("New Invoice") }
            }
        } else {
            if (!hasChannel) {
                Card(
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        "First payment will open a JIT channel via LSP.",
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodySmall
                    )
                }
                Spacer(Modifier.height(12.dp))
            }

            OutlinedTextField(
                value = amountSats,
                onValueChange = { amountSats = it.filter { c -> c.isDigit() } },
                label = { Text("Amount (sats) - leave empty for any amount") },
                modifier = Modifier.fillMaxWidth()
            )

            error?.let {
                Spacer(Modifier.height(8.dp))
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }

            Spacer(Modifier.height(16.dp))
            Button(
                onClick = {
                    isGenerating = true
                    error = null
                    scope.launch(Dispatchers.IO) {
                        try {
                            val sats = amountSats.toLongOrNull()
                            val inv = if (!hasChannel && sats != null && sats > 0) {
                                appState.nodeService.receiveViaJitChannel(sats * 1000, "Stable Channels")
                            } else if (sats != null && sats > 0) {
                                appState.nodeService.receivePayment(sats * 1000, "Stable Channels")
                            } else {
                                appState.nodeService.receiveVariablePayment("Stable Channels")
                            }
                            invoice = inv.toString()
                        } catch (e: Exception) {
                            error = e.message ?: "Failed to generate invoice"
                        }
                        isGenerating = false
                    }
                },
                enabled = !isGenerating,
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isGenerating) CircularProgressIndicator(Modifier.size(20.dp))
                else Text("Generate Invoice")
            }
        }
    }
}
