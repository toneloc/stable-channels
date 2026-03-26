package com.stablechannels.app.ui.transfer

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.lightningdevkit.ldknode.Bolt11Invoice
import org.lightningdevkit.ldknode.Offer

enum class InputType { BOLT11, BOLT12, ONCHAIN, UNKNOWN }

@Composable
fun SendScreen(appState: AppState, onDismiss: () -> Unit) {
    var input by remember { mutableStateOf("") }
    var amountSats by remember { mutableStateOf("") }
    var amountUSDStr by remember { mutableStateOf("") }
    var isSending by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val btcPrice by appState.priceService.currentPrice.collectAsState()

    val inputType = remember(input) {
        val lower = input.trim().lowercase()
        when {
            lower.startsWith("lnbc") || lower.startsWith("lntb") || lower.startsWith("lnts") -> InputType.BOLT11
            lower.startsWith("lno") -> InputType.BOLT12
            lower.startsWith("bc1") || lower.startsWith("1") || lower.startsWith("3") || lower.startsWith("tb1") -> InputType.ONCHAIN
            else -> InputType.UNKNOWN
        }
    }

    val parsedBolt11Msat = remember(input) {
        if (inputType != InputType.BOLT11) null
        else try { Bolt11Invoice.fromStr(input.trim()).amountMilliSatoshis()?.toLong() } catch (_: Exception) { null }
    }

    val isAmountlessBolt11 = inputType == InputType.BOLT11 && parsedBolt11Msat == null && input.isNotBlank()

    val manualAmountMsat: Long = run {
        if (btcPrice <= 0) return@run 0L
        val usd = amountUSDStr.toDoubleOrNull() ?: return@run 0L
        if (usd <= 0) return@run 0L
        val btc = usd / btcPrice
        (btc * Constants.SATS_IN_BTC * 1000).toLong()
    }

    val needsAmount = when {
        isAmountlessBolt11 -> manualAmountMsat == 0L
        inputType == InputType.BOLT12 || inputType == InputType.ONCHAIN -> (amountSats.toLongOrNull() ?: 0) == 0L
        else -> false
    }

    val displaySats: Long = when (inputType) {
        InputType.BOLT11 -> if ((parsedBolt11Msat ?: 0) > 0) (parsedBolt11Msat ?: 0) / 1000 else manualAmountMsat / 1000
        InputType.BOLT12, InputType.ONCHAIN -> amountSats.toLongOrNull() ?: 0
        InputType.UNKNOWN -> 0
    }

    val displayUSD = if (btcPrice > 0 && displaySats > 0) (displaySats.toDouble() / Constants.SATS_IN_BTC) * btcPrice else null

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text("Send", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(16.dp))

        if (result != null) {
            Text("Sent!", style = MaterialTheme.typography.headlineMedium, color = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.height(8.dp))
            Text(result!!, style = MaterialTheme.typography.bodySmall)
            Spacer(Modifier.height(16.dp))
            Button(onClick = onDismiss) { Text("Done") }
        } else {
            OutlinedTextField(
                value = input,
                onValueChange = { input = it },
                label = { Text("Invoice, Offer, or Address") },
                modifier = Modifier.fillMaxWidth(),
                minLines = 2
            )

            if (inputType != InputType.UNKNOWN) {
                Spacer(Modifier.height(4.dp))
                Text(
                    text = "Detected: ${inputType.name.lowercase().replaceFirstChar { it.uppercase() }}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary
                )
            }

            // Bolt11 with amount
            if (inputType == InputType.BOLT11 && (parsedBolt11Msat ?: 0) > 0) {
                displayUSD?.let {
                    Spacer(Modifier.height(8.dp))
                    Text("${it.usdFormatted()}", style = MaterialTheme.typography.labelMedium)
                }
            }

            // Amountless bolt11 — USD input
            if (isAmountlessBolt11) {
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = amountUSDStr,
                    onValueChange = { amountUSDStr = it },
                    label = { Text("Amount (USD)") },
                    modifier = Modifier.fillMaxWidth()
                )
                if (manualAmountMsat > 0) {
                    displayUSD?.let {
                        Spacer(Modifier.height(4.dp))
                        Text("~ ${it.usdFormatted()}", style = MaterialTheme.typography.labelSmall)
                    }
                }
            }

            // Bolt12 / onchain — sats input
            if (inputType == InputType.BOLT12 || inputType == InputType.ONCHAIN) {
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = amountSats,
                    onValueChange = { amountSats = it.filter { c -> c.isDigit() } },
                    label = { Text("Amount (sats)") },
                    modifier = Modifier.fillMaxWidth()
                )
                val sats = amountSats.toLongOrNull() ?: 0
                if (sats > 0 && btcPrice > 0) {
                    val usd = (sats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
                    Text("~ ${usd.usdFormatted()}", style = MaterialTheme.typography.labelSmall)
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
                            appState.ensureLSPConnected()
                            val trimmed = input.trim()
                            val price = btcPrice
                            when (inputType) {
                                InputType.BOLT11 -> {
                                    val invoice = Bolt11Invoice.fromStr(trimmed)
                                    val invoiceMsat = invoice.amountMilliSatoshis()?.toLong() ?: 0L
                                    val paymentId: String
                                    val actualMsat: Long
                                    if (invoiceMsat > 0) {
                                        paymentId = appState.nodeService.sendPayment(invoice)
                                        actualMsat = invoiceMsat
                                    } else {
                                        actualMsat = manualAmountMsat
                                        paymentId = appState.nodeService.sendPaymentUsingAmount(invoice, actualMsat)
                                    }
                                    appState.databaseService?.recordPayment(
                                        paymentId = paymentId, paymentType = "lightning",
                                        direction = "sent", amountMsat = actualMsat, btcPrice = price
                                    )
                                    result = "Payment sent"
                                }
                                InputType.BOLT12 -> {
                                    val sats = amountSats.toLongOrNull() ?: throw Exception("Enter amount")
                                    val offer = Offer.fromStr(trimmed)
                                    val paymentId = appState.nodeService.sendBolt12UsingAmount(offer, sats * 1000)
                                    appState.databaseService?.recordPayment(
                                        paymentId = paymentId, paymentType = "bolt12",
                                        direction = "sent", amountMsat = sats * 1000, btcPrice = price
                                    )
                                    result = "Bolt12 payment sent"
                                }
                                InputType.ONCHAIN -> {
                                    val sats = amountSats.toLongOrNull() ?: throw Exception("Enter amount")
                                    val hasChannel = appState.nodeService.channels.any { it.isChannelReady }
                                    if (hasChannel) {
                                        if (appState.isSpliceInFlight) throw Exception("A splice is already in progress — try again shortly")
                                        val sc = appState.stableChannel.value
                                        appState.pendingSplice = com.stablechannels.app.models.PendingSplice("out", sats, trimmed)
                                        appState.nodeService.spliceOut(sc.userChannelId, sc.counterparty, trimmed, sats)
                                        result = "Splice-out initiated"
                                    } else {
                                        val txid = appState.nodeService.sendOnchain(trimmed, sats)
                                        appState.databaseService?.recordPayment(
                                            paymentId = null, paymentType = "onchain",
                                            direction = "sent", amountMsat = sats * 1000,
                                            btcPrice = price, txid = txid, address = trimmed
                                        )
                                        result = "On-chain tx sent: $txid"
                                    }
                                }
                                InputType.UNKNOWN -> throw Exception("Enter a valid invoice, offer, or address")
                            }
                        } catch (e: Exception) {
                            error = e.message ?: "Send failed"
                        }
                        isSending = false
                    }
                },
                enabled = !isSending && input.isNotBlank() && !needsAmount,
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isSending) CircularProgressIndicator(Modifier.size(20.dp))
                else Text("Send")
            }
        }
    }
}
