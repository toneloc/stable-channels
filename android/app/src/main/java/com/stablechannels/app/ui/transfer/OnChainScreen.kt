package com.stablechannels.app.ui.transfer

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
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
        Text("On-Chain Send", style = MaterialTheme.typography.headlineSmall)
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

            OutlinedTextField(
                value = address,
                onValueChange = { address = it },
                label = { Text("Bitcoin Address") },
                modifier = Modifier.fillMaxWidth()
            )
            Spacer(Modifier.height(12.dp))

            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(checked = sendAll, onCheckedChange = { sendAll = it })
                Text("Send all (${onchainSats.satsFormatted()})")
            }

            if (!sendAll) {
                OutlinedTextField(
                    value = amountUSDStr,
                    onValueChange = { amountUSDStr = it.filter { c -> c.isDigit() || c == '.' } },
                    label = { Text("Amount (USD)") },
                    prefix = { Text("$") },
                    modifier = Modifier.fillMaxWidth()
                )
                val usd = amountUSDStr.toDoubleOrNull() ?: 0.0
                val satsFromUSD = if (btcPrice > 0 && usd > 0) (usd / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
                if (satsFromUSD > 0) {
                    Text("~ ${satsFromUSD.satsFormatted()}", style = MaterialTheme.typography.labelSmall)
                }
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
