package com.stablechannels.app.ui.trade

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PendingTradePayment
import com.stablechannels.app.models.USD
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.Locale

@Composable
fun SellScreen(appState: AppState, prefillAmountUSD: Double = 0.0, onDismiss: () -> Unit) {
    var step by remember { mutableStateOf(TradeStep.AMOUNT) }
    var amountText by remember { mutableStateOf(if (prefillAmountUSD > 0) String.format(Locale.US, "%.2f", prefillAmountUSD) else "") }
    var error by remember { mutableStateOf<String?>(null) }
    var isExecuting by remember { mutableStateOf(false) }
    var pendingPaymentId by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    val sc by appState.stableChannel.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val lightningSats by appState.lightningBalanceSats.collectAsState()
    val stableSats = if (btcPrice > 0) (sc.expectedUSD.amount / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
    val nativeSatsDisplay = if (lightningSats > stableSats) lightningSats - stableSats else 0L
    val maxSellUSD = if (btcPrice > 0) (nativeSatsDisplay.toDouble() / Constants.SATS_IN_BTC) * btcPrice else 0.0
    val amountUSD = amountText.toDoubleOrNull() ?: 0.0
    val feeUSD = amountUSD * 0.01
    val btcAmount = if (btcPrice > 0) (amountUSD - feeUSD) / btcPrice else 0.0

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        when (step) {
            TradeStep.AMOUNT -> {
                Text("Sell BTC", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(4.dp))
                Text("Max: ${maxSellUSD.usdFormatted()}", style = MaterialTheme.typography.labelMedium)
                Spacer(Modifier.height(16.dp))

                OutlinedTextField(
                    value = amountText,
                    onValueChange = { amountText = it },
                    label = { Text("Amount (USD)") },
                    prefix = if (amountText.isNotEmpty()) {{ Text("$", fontSize = 16.sp, fontWeight = FontWeight.Medium) }} else null,
                    modifier = Modifier.fillMaxWidth()
                )

                if (amountUSD > 0 && btcPrice > 0) {
                    Spacer(Modifier.height(4.dp))
                    Text(
                        "~ ${String.format(Locale.US, "%.8f", btcAmount)} BTC",
                        style = MaterialTheme.typography.labelSmall
                    )
                }

                error?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error)
                }

                Spacer(Modifier.height(16.dp))
                Button(
                    onClick = {
                        if (amountUSD <= 0 || amountUSD > maxSellUSD) {
                            error = "Enter an amount between $0 and ${maxSellUSD.usdFormatted()}"
                        } else {
                            error = null
                            step = TradeStep.CONFIRM
                        }
                    },
                    modifier = Modifier.fillMaxWidth()
                ) { Text("Continue") }
            }

            TradeStep.CONFIRM -> {
                Text("Confirm Sell", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(16.dp))

                ConfirmRow("Amount", amountUSD.usdFormatted())
                ConfirmRow("Fee (1%)", feeUSD.usdFormatted())
                ConfirmRow("BTC Price", btcPrice.usdFormatted())
                ConfirmRow("You receive", (amountUSD - feeUSD).usdFormatted())

                error?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error)
                }

                Spacer(Modifier.height(16.dp))
                Button(
                    onClick = {
                        isExecuting = true
                        error = null
                        scope.launch(Dispatchers.IO) {
                            try {
                                appState.ensureLSPConnected()
                                val totalUSD = USD.fromBitcoin(sc.stableReceiverBTC, btcPrice).amount
                                val result = appState.tradeService?.executeSell(sc, amountUSD, feeUSD, btcPrice, totalUSD)
                                    ?: throw Exception("Trade service unavailable")
                                val tradeDbId = appState.databaseService?.recordTrade(
                                    channelId = sc.channelId, action = "sell",
                                    amountUSD = amountUSD, amountBTC = result.btcAmount,
                                    btcPrice = btcPrice, feeUSD = feeUSD,
                                    paymentId = result.paymentId, status = "pending"
                                ) ?: 0
                                appState.addPendingTradePayment(result.paymentId, PendingTradePayment(
                                    newExpectedUSD = result.newExpectedUSD,
                                    price = btcPrice,
                                    tradeDbId = tradeDbId,
                                    action = "sell"
                                ))
                                pendingPaymentId = result.paymentId
                                appState.setStatus(String.format(Locale.US, "Sell pending (fee: $%.2f)", feeUSD))
                                step = TradeStep.DONE
                            } catch (e: Exception) {
                                error = e.message ?: "Trade failed"
                            }
                            isExecuting = false
                        }
                    },
                    enabled = !isExecuting,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    if (isExecuting) CircularProgressIndicator(Modifier.size(20.dp))
                    else Text("Confirm Sell")
                }
                Spacer(Modifier.height(8.dp))
                TextButton(onClick = { step = TradeStep.AMOUNT }) { Text("Back") }
            }

            TradeStep.DONE -> {
                val pendingPayments by appState.pendingTradePayments.collectAsState()
                val isConfirmed = pendingPaymentId != null && !pendingPayments.containsKey(pendingPaymentId)

                if (isConfirmed) {
                    Icon(
                        Icons.Filled.CheckCircle,
                        contentDescription = "Confirmed",
                        tint = Color(0xFF4CAF50),
                        modifier = Modifier.size(48.dp)
                    )
                    Spacer(Modifier.height(8.dp))
                    Text("Trade Confirmed", style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Your sell order has been confirmed.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                } else {
                    CircularProgressIndicator(Modifier.size(48.dp))
                    Spacer(Modifier.height(8.dp))
                    Text("Trade Pending", style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Your sell order is being processed. Balance will update when the payment confirms.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
                Spacer(Modifier.height(16.dp))
                Button(onClick = onDismiss) { Text("Done") }
            }
        }
    }
}
