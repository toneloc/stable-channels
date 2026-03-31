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
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.home.FundWalletScreen
import com.stablechannels.app.ui.home.generateQRCode
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReceiveScreen(appState: AppState, onDismiss: () -> Unit) {
    var amountUSD by remember { mutableStateOf("") }
    var invoice by remember { mutableStateOf<String?>(null) }
    var invoiceAmountSats by remember { mutableStateOf<Long?>(null) }
    var isGenerating by remember { mutableStateOf(false) }
    var isCopied by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var showOnChain by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val clipboardManager = LocalClipboardManager.current
    val btcPrice by appState.priceService.currentPrice.collectAsState()

    val hasChannel = appState.nodeService.channels.any { it.isChannelReady }

    val enteredUSD = amountUSD.toDoubleOrNull() ?: 0.0
    val enteredSats = if (btcPrice > 0 && enteredUSD > 0) {
        (enteredUSD / btcPrice * Constants.SATS_IN_BTC).toLong()
    } else 0L

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
        Text("Receive", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(16.dp))

        // Lightning vs On-Chain toggle
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            FilledTonalButton(
                onClick = { showOnChain = false },
                modifier = Modifier.weight(1f),
                colors = ButtonDefaults.filledTonalButtonColors(
                    containerColor = if (!showOnChain) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = if (!showOnChain) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant
                )
            ) { Text("Lightning") }
            FilledTonalButton(
                onClick = { showOnChain = true },
                modifier = Modifier.weight(1f),
                colors = ButtonDefaults.filledTonalButtonColors(
                    containerColor = if (showOnChain) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = if (showOnChain) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant
                )
            ) { Text("On-Chain") }
        }
        Spacer(Modifier.height(16.dp))

        if (invoice != null) {
            val inv = invoice!!

            // Amount summary
            if (invoiceAmountSats != null && invoiceAmountSats!! > 0) {
                if (btcPrice > 0) {
                    val usd = invoiceAmountSats!!.toDouble() / Constants.SATS_IN_BTC * btcPrice
                    Text(usd.usdFormatted(), style = MaterialTheme.typography.headlineMedium, fontWeight = androidx.compose.ui.text.font.FontWeight.Bold)
                }
                Text(
                    invoiceAmountSats!!.btcSpacedFormatted(),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(12.dp))
            }

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
                    invoiceAmountSats = null
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
                        "First payment — a channel will be opened automatically via LSP",
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodySmall
                    )
                }
                Spacer(Modifier.height(12.dp))
            }

            Text("Amount (USD)", style = MaterialTheme.typography.titleSmall)
            Spacer(Modifier.height(8.dp))

            OutlinedTextField(
                value = amountUSD,
                onValueChange = { amountUSD = it.filter { c -> c.isDigit() || c == '.' } },
                placeholder = { Text("0.00") },
                prefix = { Text("$") },
                modifier = Modifier.fillMaxWidth()
            )

            if (enteredSats > 0) {
                Spacer(Modifier.height(4.dp))
                Text(
                    "${enteredSats.btcSpacedFormatted()}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

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
                            val sats = enteredSats
                            val inv = if (!hasChannel && sats > 0) {
                                appState.nodeService.receiveViaJitChannel(sats * 1000, "Stable Channels")
                            } else if (sats > 0) {
                                appState.nodeService.receivePayment(sats * 1000, "Stable Channels")
                            } else {
                                appState.nodeService.receiveVariablePayment("Stable Channels")
                            }
                            invoiceAmountSats = if (sats > 0) sats else null
                            invoice = inv.toString()
                        } catch (e: Exception) {
                            error = e.message ?: "Failed to generate invoice"
                        }
                        isGenerating = false
                    }
                },
                enabled = !isGenerating && enteredSats > 0,
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isGenerating) CircularProgressIndicator(Modifier.size(20.dp))
                else Text("Generate Invoice")
            }

            if (hasChannel) {
                Spacer(Modifier.height(8.dp))
                OutlinedButton(
                    onClick = {
                        isGenerating = true
                        error = null
                        scope.launch(Dispatchers.IO) {
                            try {
                                val inv = appState.nodeService.receiveVariablePayment("Stable Channels")
                                invoiceAmountSats = null
                                invoice = inv.toString()
                            } catch (e: Exception) {
                                error = e.message ?: "Failed to generate invoice"
                            }
                            isGenerating = false
                        }
                    },
                    enabled = !isGenerating,
                    modifier = Modifier.fillMaxWidth()
                ) { Text("Any Amount") }
            }
        }
    }
}
