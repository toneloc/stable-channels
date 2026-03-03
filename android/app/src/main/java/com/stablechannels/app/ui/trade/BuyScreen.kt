package com.stablechannels.app.ui.trade

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PendingTradePayment
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.Locale

enum class TradeStep { AMOUNT, CONFIRM, DONE }

@Composable
fun BuyScreen(appState: AppState, onDismiss: () -> Unit) {
    var step by remember { mutableStateOf(TradeStep.AMOUNT) }
    var amountText by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var isExecuting by remember { mutableStateOf(false) }
    var pendingPaymentId by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    val sc by appState.stableChannel.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val maxBuyUSD = sc.expectedUSD.amount
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
                Text("Buy BTC", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(4.dp))
                Text("Max: ${maxBuyUSD.usdFormatted()}", style = MaterialTheme.typography.labelMedium)
                Spacer(Modifier.height(16.dp))

                OutlinedTextField(
                    value = amountText,
                    onValueChange = { amountText = it },
                    label = { Text("Amount (USD)") },
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
                        if (amountUSD <= 0 || amountUSD > maxBuyUSD) {
                            error = "Enter an amount between $0 and ${maxBuyUSD.usdFormatted()}"
                        } else {
                            error = null
                            step = TradeStep.CONFIRM
                        }
                    },
                    modifier = Modifier.fillMaxWidth()
                ) { Text("Continue") }
            }

            TradeStep.CONFIRM -> {
                Text("Confirm Buy", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(16.dp))

                ConfirmRow("Amount", amountUSD.usdFormatted())
                ConfirmRow("Fee (1%)", feeUSD.usdFormatted())
                ConfirmRow("BTC Price", btcPrice.usdFormatted())
                ConfirmRow("You receive", String.format(Locale.US, "%.8f BTC", btcAmount))

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
                                val result = appState.tradeService?.executeBuy(sc, amountUSD, feeUSD, btcPrice)
                                    ?: throw Exception("Trade service unavailable")
                                val tradeDbId = appState.databaseService?.recordTrade(
                                    channelId = sc.channelId, action = "buy",
                                    amountUSD = amountUSD, amountBTC = result.btcAmount,
                                    btcPrice = btcPrice, feeUSD = feeUSD,
                                    paymentId = result.paymentId, status = "pending"
                                ) ?: 0
                                appState.addPendingTradePayment(result.paymentId, PendingTradePayment(
                                    newExpectedUSD = result.newExpectedUSD,
                                    price = btcPrice,
                                    tradeDbId = tradeDbId,
                                    action = "buy"
                                ))
                                pendingPaymentId = result.paymentId
                                appState.setStatus(String.format(Locale.US, "Buy pending (fee: $%.2f)", feeUSD))
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
                    else Text("Confirm Buy")
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
                        "Your buy order has been confirmed.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                } else {
                    CircularProgressIndicator(Modifier.size(48.dp))
                    Spacer(Modifier.height(8.dp))
                    Text("Trade Pending", style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Your buy order is being processed. Balance will update when the payment confirms.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
                Spacer(Modifier.height(16.dp))
                Button(onClick = onDismiss) { Text("Done") }
            }
        }
    }
}

@Composable
fun ConfirmRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}
