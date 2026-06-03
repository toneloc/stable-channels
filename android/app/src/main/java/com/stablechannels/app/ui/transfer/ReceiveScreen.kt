package com.stablechannels.app.ui.transfer

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
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
        FundWalletScreen(appState, onBack = { showOnChain = false })
        return
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Toolbar (Cancel button, centered title, top-right Onchain button)
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            TextButton(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.CenterStart),
                colors = ButtonDefaults.textButtonColors(
                    containerColor = if (isSystemInDarkTheme()) {
                        MaterialTheme.colorScheme.surfaceVariant
                    } else {
                        Color(0xFFE5E5EA)
                    },
                    contentColor = MaterialTheme.colorScheme.primary
                ),
                shape = RoundedCornerShape(20.dp),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
            ) {
                Text("Cancel", style = MaterialTheme.typography.bodyMedium)
            }
            Text(
                text = "Receive",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.align(Alignment.Center)
            )
            TextButton(
                onClick = { showOnChain = true },
                modifier = Modifier.align(Alignment.CenterEnd),
                colors = ButtonDefaults.textButtonColors(
                    containerColor = if (isSystemInDarkTheme()) {
                        MaterialTheme.colorScheme.surfaceVariant
                    } else {
                        Color(0xFFE5E5EA)
                    },
                    contentColor = MaterialTheme.colorScheme.primary
                ),
                shape = RoundedCornerShape(20.dp),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
            ) {
                Text("Onchain", style = MaterialTheme.typography.bodyMedium)
            }
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
                Text(
                    "$${Constants.MAX_CHANNEL_USD.toInt()} Maximum",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(12.dp))
            }

            Text("Amount (USD)", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(12.dp))
 
            Row(
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("$", fontSize = 44.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.width(2.dp))
                BasicTextField(
                    value = amountUSD,
                    onValueChange = { amountUSD = it.filter { c -> c.isDigit() || c == '.' } },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    textStyle = TextStyle(
                        fontSize = 44.sp,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.onSurface,
                        textAlign = TextAlign.Start
                    ),
                    singleLine = true,
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    modifier = Modifier.width(IntrinsicSize.Min),
                    decorationBox = { innerTextField ->
                        Box(contentAlignment = Alignment.CenterStart) {
                            if (amountUSD.isEmpty()) {
                                Text(
                                    text = "0.00",
                                    style = TextStyle(
                                        fontSize = 44.sp,
                                        fontWeight = FontWeight.Bold,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                                        textAlign = TextAlign.Start
                                    )
                                )
                            }
                            innerTextField()
                        }
                    }
                )
            }

            if (enteredSats > 0) {
                Spacer(Modifier.height(4.dp))
                Text(
                    "${enteredSats.btcSpacedFormatted()}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            if (!hasChannel && enteredUSD > Constants.MAX_CHANNEL_USD) {
                Spacer(Modifier.height(4.dp))
                Text(
                    "Amount exceeds $${Constants.MAX_CHANNEL_USD.toInt()} channel limit",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall
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
                enabled = !isGenerating && enteredSats > 0 && (hasChannel || enteredUSD <= Constants.MAX_CHANNEL_USD),
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
