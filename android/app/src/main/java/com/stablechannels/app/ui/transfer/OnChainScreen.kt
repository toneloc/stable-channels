package com.stablechannels.app.ui.transfer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PendingSplice
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@Composable
fun OnChainSendScreen(appState: AppState, onDismiss: () -> Unit) {
    var address by remember { mutableStateOf("") }
    var amountUSDStr by remember { mutableStateOf("") }
    var sendAll by remember { mutableStateOf(false) }
    var isSending by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val onchainSats by appState.onchainBalanceSats.collectAsState()

    val hasChannel = appState.nodeService.channels.any { it.isChannelReady }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Toolbar (Cancel button, centered title)
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            if (result == null) {
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
            }
            Text(
                text = "Onchain Send",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.align(Alignment.Center)
            )
        }
        Spacer(Modifier.height(16.dp))

        if (result != null) {
            Text("Sent!", style = MaterialTheme.typography.headlineMedium, color = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.height(8.dp))
            Text(result!!, style = MaterialTheme.typography.bodySmall)
            Spacer(Modifier.height(16.dp))
            Button(onClick = onDismiss) { Text("Done") }
        } else {
            if (hasChannel && !sendAll) {
                Card(
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        "Will use splice-out via your Lightning channel for faster settlement.",
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodySmall
                    )
                }
                Spacer(Modifier.height(12.dp))
            }

            if (appState.isChannelClosing) {
                Card(
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.errorContainer),
                    modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp)
                ) {
                    Text(
                        "Channel is closing — you should sweep your remaining onchain funds.",
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onErrorContainer
                    )
                }
                Spacer(Modifier.height(8.dp))
            }

            OutlinedTextField(
                value = address,
                onValueChange = { address = it },
                label = { Text("Bitcoin Address") },
                modifier = Modifier.fillMaxWidth()
            )
            Spacer(Modifier.height(16.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("Amount (USD)", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                TextButton(
                    onClick = { sendAll = !sendAll },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)
                ) {
                    Text(if (sendAll) "Enter Amount" else "Send Max", style = MaterialTheme.typography.labelMedium)
                }
            }
            Spacer(Modifier.height(12.dp))

            if (!sendAll) {
                Row(
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text("$", fontSize = 44.sp, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.width(2.dp))
                    BasicTextField(
                        value = amountUSDStr,
                        onValueChange = { amountUSDStr = it.filter { c -> c.isDigit() || c == '.' } },
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
                                if (amountUSDStr.isEmpty()) {
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

                val usd = amountUSDStr.toDoubleOrNull() ?: 0.0
                val satsFromUSD = if (btcPrice > 0 && usd > 0) (usd / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
                if (satsFromUSD > 0) {
                    Spacer(Modifier.height(4.dp))
                    Text("~ ${satsFromUSD.satsFormatted()} sats", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            } else {
                Text(
                    text = "Send All (${onchainSats.satsFormatted()} sats)",
                    fontSize = 22.sp,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.primary
                )
            }

            error?.let {
                Spacer(Modifier.height(8.dp))
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }

            Spacer(Modifier.height(16.dp))
            Button(
                onClick = {
                    isSending = true
                    error = null
                    scope.launch(Dispatchers.IO) {
                        try {
                            val addr = address.trim()
                            val price = btcPrice
                            if (sendAll) {
                                val txid = appState.nodeService.sendAllOnchain(addr)
                                val sendSats = onchainSats
                                appState.databaseService?.recordPayment(
                                    paymentId = txid, paymentType = "onchain", direction = "sent",
                                    amountMsat = sendSats * 1000,
                                    amountUSD = if (price > 0) (sendSats.toDouble() / Constants.SATS_IN_BTC) * price else null,
                                    btcPrice = if (price > 0) price else null,
                                    txid = txid, address = addr
                                )
                                result = "All funds sent. TXID: $txid"
                            } else {
                                val usd = amountUSDStr.toDoubleOrNull() ?: throw Exception("Enter amount")
                                val sats = if (price > 0) (usd / price * Constants.SATS_IN_BTC).toLong() else throw Exception("No price available")
                                if (hasChannel) {
                                    if (appState.isSpliceInFlight) throw Exception("A splice is already in progress — try again shortly")
                                    val sc = appState.stableChannel.value
                                    appState.pendingSplice = PendingSplice("out", sats, addr)
                                    appState.nodeService.spliceOut(sc.userChannelId, sc.counterparty, addr, sats)
                                    result = "Splice-out initiated for $sats sats"
                                } else {
                                    val txid = appState.nodeService.sendOnchain(addr, sats)
                                    appState.databaseService?.recordPayment(
                                        paymentId = txid, paymentType = "onchain", direction = "sent",
                                        amountMsat = sats * 1000,
                                        amountUSD = if (price > 0) (sats.toDouble() / Constants.SATS_IN_BTC) * price else null,
                                        btcPrice = if (price > 0) price else null,
                                        txid = txid, address = addr
                                    )
                                    result = "Sent. TXID: $txid"
                                }
                            }
                        } catch (e: Exception) {
                            error = e.message ?: "Send failed"
                        }
                        isSending = false
                    }
                },
                enabled = !isSending && address.isNotBlank(),
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isSending) CircularProgressIndicator(Modifier.size(20.dp))
                else Text("Send")
            }
        }
    }
}
