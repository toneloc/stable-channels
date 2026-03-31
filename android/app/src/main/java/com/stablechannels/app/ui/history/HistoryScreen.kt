package com.stablechannels.app.ui.history

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.CallMade
import androidx.compose.material.icons.automirrored.filled.CallReceived
import androidx.compose.material.icons.filled.CompareArrows
import androidx.compose.material.icons.filled.ElectricBolt
import androidx.compose.material.icons.filled.ShoppingCart
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.models.TradeRecord
import com.stablechannels.app.util.relativeString
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted

@Composable
fun HistoryScreen(appState: AppState, modifier: Modifier = Modifier) {
    var selectedSegment by remember { mutableIntStateOf(0) }
    var trades by remember { mutableStateOf<List<TradeRecord>>(emptyList()) }
    var payments by remember { mutableStateOf<List<PaymentRecord>>(emptyList()) }
    var selectedTrade by remember { mutableStateOf<TradeRecord?>(null) }
    var selectedPayment by remember { mutableStateOf<PaymentRecord?>(null) }

    fun loadHistory() {
        trades = appState.databaseService?.getRecentTrades() ?: emptyList()
        payments = appState.databaseService?.getRecentPayments() ?: emptyList()
    }

    LaunchedEffect(Unit) { loadHistory() }

    Column(modifier = modifier.fillMaxSize().padding(16.dp)) {
        Text("History", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(12.dp))

        // Segment tabs
        TabRow(selectedTabIndex = selectedSegment) {
            Tab(selected = selectedSegment == 0, onClick = { selectedSegment = 0 }) { Text("Trades", Modifier.padding(12.dp)) }
            Tab(selected = selectedSegment == 1, onClick = { selectedSegment = 1 }) { Text("Payments", Modifier.padding(12.dp)) }
        }

        Spacer(Modifier.height(8.dp))

        if (selectedSegment == 0 && trades.isEmpty()) {
            EmptyStateView(
                icon = Icons.Default.CompareArrows,
                title = "No Trades",
                description = "Buy or sell BTC to see trades here."
            )
        } else if (selectedSegment == 1 && payments.isEmpty()) {
            EmptyStateView(
                icon = Icons.Default.ElectricBolt,
                title = "No Payments",
                description = "Send or receive payments to see history here."
            )
        } else {
            LazyColumn {
                if (selectedSegment == 0) {
                    items(trades) { trade ->
                        TradeRow(trade) { selectedTrade = trade }
                    }
                } else {
                    items(payments) { payment ->
                        PaymentRow(payment) { selectedPayment = payment }
                    }
                }
            }
        }
    }

    // Detail dialogs
    selectedTrade?.let { trade ->
        TradeDetailDialog(trade) { selectedTrade = null }
    }
    selectedPayment?.let { payment ->
        PaymentDetailDialog(payment) { selectedPayment = null }
    }
}

@Composable
fun TradeRow(trade: TradeRecord, onClick: () -> Unit) {
    val isBuy = trade.action == "buy"
    val icon = if (isBuy) Icons.Default.ShoppingCart else Icons.Default.Sell
    val color = if (isBuy) Color(0xFFF59E0B) else Color(0xFF8B5CF6)

    ListItem(
        modifier = Modifier.clickable(onClick = onClick),
        leadingContent = { Icon(icon, contentDescription = null, tint = color) },
        headlineContent = { Text(if (isBuy) "Buy BTC" else "Sell BTC") },
        supportingContent = { Text(trade.date.relativeString()) },
        trailingContent = {
            Column(horizontalAlignment = Alignment.End) {
                Text(trade.amountUSD.usdFormatted())
                Text(
                    trade.status,
                    style = MaterialTheme.typography.labelSmall,
                    color = when (trade.status) {
                        "completed" -> Color(0xFF10B981)
                        "failed" -> MaterialTheme.colorScheme.error
                        else -> MaterialTheme.colorScheme.onSurfaceVariant
                    }
                )
            }
        }
    )
}

@Composable
fun PaymentRow(payment: PaymentRecord, onClick: () -> Unit) {
    val isIncoming = payment.isIncoming
    val icon = if (isIncoming) Icons.AutoMirrored.Filled.CallReceived else Icons.AutoMirrored.Filled.CallMade
    val color = if (isIncoming) Color(0xFF10B981) else Color(0xFF3B82F6)
    val typeLabel = when (payment.paymentType) {
        "stability" -> "Stability"
        "splice_in" -> "Splice In"
        "splice_out" -> "Splice Out"
        "onchain" -> "On-chain"
        "channel_close" -> "Channel Close"
        else -> "Lightning"
    }

    ListItem(
        modifier = Modifier.clickable(onClick = onClick),
        leadingContent = { Icon(icon, contentDescription = null, tint = color) },
        headlineContent = { Text(if (isIncoming) "Received" else "Sent") },
        supportingContent = { Text("$typeLabel  ${payment.date.relativeString()}") },
        trailingContent = {
            Column(horizontalAlignment = Alignment.End) {
                Text(payment.amountUSD?.usdFormatted() ?: payment.amountSats.satsFormatted())
                Text(
                    payment.status,
                    style = MaterialTheme.typography.labelSmall,
                    color = when (payment.status) {
                        "completed" -> Color(0xFF10B981)
                        "failed" -> MaterialTheme.colorScheme.error
                        else -> MaterialTheme.colorScheme.onSurfaceVariant
                    }
                )
            }
        }
    )
}

@Composable
private fun EmptyStateView(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    description: String
) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 80.dp),
        contentAlignment = Alignment.Center
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(
                icon,
                contentDescription = null,
                modifier = Modifier.size(48.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
            )
            Spacer(Modifier.height(12.dp))
            Text(
                title,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurface
            )
            Spacer(Modifier.height(4.dp))
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center
            )
        }
    }
}
