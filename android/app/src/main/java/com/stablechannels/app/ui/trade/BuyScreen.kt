package com.stablechannels.app.ui.trade

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PendingTradePayment
import com.stablechannels.app.util.usdFormatted
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.Locale

enum class TradeStep { AMOUNT, CONFIRM, DONE }

@Composable
fun BuyScreen(appState: AppState, prefillAmountUSD: Double = 0.0, onDismiss: () -> Unit) {
    var step by remember { mutableStateOf(TradeStep.AMOUNT) }
    var amountText by remember { mutableStateOf(if (prefillAmountUSD > 0) String.format(Locale.US, "%.2f", prefillAmountUSD) else "") }
    var error by remember { mutableStateOf<String?>(null) }
    var isExecuting by remember { mutableStateOf(false) }
    var pendingPaymentId by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    val sc by appState.stableChannel.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val maxBuyUSD = sc.expectedUSD.amount
    val amountUSD = amountText.toDoubleOrNull() ?: 0.0
    val feeUSD = amountUSD * Constants.STABLE_CHANNEL_TRADE_FEE_RATE
    val feeLabel = String.format(Locale.US, "Fee (%.0f%%)", Constants.STABLE_CHANNEL_TRADE_FEE_RATE * 100)
    val btcAmount = if (btcPrice > 0) (amountUSD - feeUSD) / btcPrice else 0.0

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Top Toolbar (Cancel / Title)
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            if (step != TradeStep.DONE) {
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
                    shape = androidx.compose.foundation.shape.RoundedCornerShape(20.dp),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                ) {
                    Text("Cancel", style = MaterialTheme.typography.bodyMedium)
                }
            }
            Text(
                text = if (step == TradeStep.CONFIRM) "Review USD -> BTC" else "USD → BTC",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.align(Alignment.Center)
            )
        }
        Spacer(Modifier.height(16.dp))

        when (step) {
            TradeStep.AMOUNT -> {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("How much USD to convert to BTC?", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(
                        onClick = { amountText = String.format(Locale.US, "%.2f", maxBuyUSD) },
                        colors = ButtonDefaults.textButtonColors(
                            containerColor = if (isSystemInDarkTheme()) {
                                MaterialTheme.colorScheme.surfaceVariant
                            } else {
                                androidx.compose.ui.graphics.Color(0xFFE5E5EA)
                            },
                            contentColor = MaterialTheme.colorScheme.primary
                        ),
                        shape = androidx.compose.foundation.shape.RoundedCornerShape(20.dp),
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                    ) {
                        Text("Max", style = MaterialTheme.typography.labelMedium)
                    }
                }
                Spacer(Modifier.height(12.dp))

                Row(
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text("$", fontSize = 44.sp, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.width(2.dp))
                    BasicTextField(
                        value = amountText,
                        onValueChange = { amountText = it.filter { c -> c.isDigit() || c == '.' } },
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
                                if (amountText.isEmpty()) {
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

                if (amountUSD > 0 && btcPrice > 0) {
                    Spacer(Modifier.height(4.dp))
                    Text(
                        "~ ${String.format(Locale.US, "%.8f", btcAmount)}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }

                Spacer(Modifier.height(8.dp))
                Text("Max: ${maxBuyUSD.usdFormatted()}", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)

                error?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
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
                Card(
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)),
                    shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Column(modifier = Modifier.padding(16.dp)) {
                        ConfirmRow("Amount", amountUSD.usdFormatted())
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        ConfirmRow(feeLabel, feeUSD.usdFormatted())
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        ConfirmRow("BTC Price", btcPrice.usdFormatted())
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        ConfirmRow("You receive", Math.round(btcAmount * Constants.SATS_IN_BTC).btcSpacedFormatted() + " BTC")
                    }
                }

                error?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }

                Spacer(Modifier.height(24.dp))
                Button(
                    onClick = {
                        isExecuting = true
                        error = null
                        scope.launch(Dispatchers.IO) {
                            try {
                                appState.ensureLSPConnected()
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
                                appState.setStatus(String.format(Locale.US, "Order pending (fee: $%.2f)", feeUSD))
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
                    else Text("Confirm Order")
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
                        tint = Color(0xFF10B981),
                        modifier = Modifier.size(48.dp)
                    )
                    Spacer(Modifier.height(8.dp))
                    Text("Order Confirmed", style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Your order has been confirmed.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                } else {
                    CircularProgressIndicator(Modifier.size(48.dp))
                    Spacer(Modifier.height(8.dp))
                    Text("Order Pending", style = MaterialTheme.typography.headlineMedium)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Your order is being processed. Balance will update when the payment confirms.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
                Spacer(Modifier.height(16.dp))
                if (isConfirmed) {
                    Button(onClick = onDismiss) { Text("Done") }
                }
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
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
    }
}
