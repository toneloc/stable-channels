package com.stablechannels.app.ui.history

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowCircleDown
import androidx.compose.material.icons.filled.ArrowCircleUp
import androidx.compose.material.icons.filled.ElectricBolt
import androidx.compose.material.icons.filled.SwapHoriz
import androidx.compose.material.icons.filled.TrendingDown
import androidx.compose.material.icons.filled.TrendingUp
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.models.TradeRecord
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.relativeString
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HistoryScreen(appState: AppState, modifier: Modifier = Modifier) {
    var selectedSegment by remember { mutableIntStateOf(0) }
    var trades by remember { mutableStateOf<List<TradeRecord>>(emptyList()) }
    var payments by remember { mutableStateOf<List<PaymentRecord>>(emptyList()) }
    var selectedTrade by remember { mutableStateOf<TradeRecord?>(null) }
    var selectedPayment by remember { mutableStateOf<PaymentRecord?>(null) }
    val currentPrice by appState.priceService.currentPrice.collectAsState()

    fun loadHistory() {
        trades = appState.databaseService?.getRecentTrades() ?: emptyList()
        payments = appState.databaseService?.getRecentPayments() ?: emptyList()
    }

    LaunchedEffect(Unit) { loadHistory() }

    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        // Title — same position as Settings and Home
        Text(
            text = "History",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth()
        )
        Spacer(Modifier.height(12.dp))

        // Segmented control (like iOS Picker .segmented)
        val isDark = MaterialTheme.colorScheme.background == Color(0xFF000000)
        val activeBg = if (isDark) Color(0xFF3A3A3C) else Color.White
        val inactiveBg = if (isDark) Color(0xFF1C1C1E) else Color(0xFFE5E5EA)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(36.dp)
                .background(inactiveBg, shape = RoundedCornerShape(8.dp))
                .padding(2.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight()
                    .background(
                        color = if (selectedSegment == 0) activeBg else Color.Transparent,
                        shape = RoundedCornerShape(6.dp)
                    )
                    .clickable { selectedSegment = 0 },
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = "Orders",
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                    color = if (selectedSegment == 0) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight()
                    .background(
                        color = if (selectedSegment == 1) activeBg else Color.Transparent,
                        shape = RoundedCornerShape(6.dp)
                    )
                    .clickable { selectedSegment = 1 },
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = "Payments",
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                    color = if (selectedSegment == 1) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

            Spacer(Modifier.height(16.dp))

            if (selectedSegment == 0 && trades.isEmpty()) {
                EmptyStateView(
                    icon = Icons.Default.SwapHoriz,
                    title = "No Orders",
                    description = "Convert BTC to see orders here."
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
                        itemsIndexed(trades) { index, trade ->
                            TradeRow(trade) { selectedTrade = trade }
                            if (index < trades.lastIndex) {
                                HorizontalDivider(
                                    modifier = Modifier.padding(start = 68.dp),
                                    color = MaterialTheme.colorScheme.outlineVariant,
                                    thickness = 0.5.dp
                                )
                            }
                        }
                    } else {
                        itemsIndexed(payments) { index, payment ->
                            PaymentRow(payment, currentPrice) { selectedPayment = payment }
                            if (index < payments.lastIndex) {
                                HorizontalDivider(
                                    modifier = Modifier.padding(start = 68.dp),
                                    color = MaterialTheme.colorScheme.outlineVariant,
                                    thickness = 0.5.dp
                                )
                            }
                        }
                    }
                    // Bottom padding for nav bar
                    item { Spacer(Modifier.height(80.dp)) }
                }
            }
        }

    // Detail bottom sheets
    selectedTrade?.let { trade ->
        OrderDetailBottomSheet(trade) { selectedTrade = null }
    }
    selectedPayment?.let { payment ->
        PaymentDetailBottomSheet(payment, currentPrice) { selectedPayment = null }
    }
}

@Composable
private fun TradeRow(trade: TradeRecord, onClick: () -> Unit) {
    val isBuy = trade.action == "buy"
    val icon = if (isBuy) Icons.Default.TrendingUp else Icons.Default.TrendingDown
    val iconColor = if (isBuy) Color(0xFFF59E0B) else Color(0xFF8B5CF6)

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 12.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        // Icon with colored background
        Surface(
            shape = RoundedCornerShape(10.dp),
            color = iconColor.copy(alpha = 0.12f),
            modifier = Modifier.size(40.dp)
        ) {
            Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                Icon(icon, contentDescription = null, tint = iconColor, modifier = Modifier.size(22.dp))
            }
        }

        Spacer(Modifier.width(12.dp))

        // Title + time
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = if (isBuy) "USD → BTC" else "BTC → USD",
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium
            )
            Text(
                text = trade.date.relativeString(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }

        // Amount + status
        Column(horizontalAlignment = Alignment.End) {
            Text(
                text = trade.amountUSD.usdFormatted(),
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium
            )
            StatusBadge(trade.status)
        }
    }
}

@Composable
private fun PaymentRow(payment: PaymentRecord, currentPrice: Double, onClick: () -> Unit) {
    val isIncoming = payment.isIncoming
    val icon = if (isIncoming) Icons.Default.ArrowCircleDown else Icons.Default.ArrowCircleUp
    val iconColor = if (isIncoming) Color(0xFF10B981) else Color(0xFF3B82F6)
    val typeLabel = when (payment.paymentType) {
        "stability" -> "Settlement"
        "lightning" -> "Lightning"
        "splice_in" -> "Splice In"
        "splice_out" -> "Splice Out"
        "onchain" -> "Onchain"
        "channel_close" -> "Channel Close"
        "bolt12" -> "Bolt12"
        else -> payment.paymentType
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 12.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        // Icon with colored background
        Surface(
            shape = RoundedCornerShape(10.dp),
            color = iconColor.copy(alpha = 0.12f),
            modifier = Modifier.size(40.dp)
        ) {
            Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                Icon(icon, contentDescription = null, tint = iconColor, modifier = Modifier.size(22.dp))
            }
        }

        Spacer(Modifier.width(12.dp))

        // Title + type + time
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = if (isIncoming) "Received" else "Sent",
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium
            )
            Text(
                text = "$typeLabel · ${payment.date.relativeString()}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }

        // Amount + status
        Column(horizontalAlignment = Alignment.End) {
            val displayUsd = payment.amountUSD ?: run {
                val price = payment.btcPrice?.takeIf { it > 0.0 } ?: currentPrice.takeIf { it > 0.0 }
                price?.let { (payment.amountSats.toDouble() / Constants.SATS_IN_BTC) * it }
            }
            val amountText = displayUsd?.usdFormatted() ?: "${payment.amountSats.satsFormatted()} sats"
            Text(
                text = (if (isIncoming) "+" else "-") + amountText,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
                color = if (isIncoming) Color(0xFF10B981) else MaterialTheme.colorScheme.onSurface
            )
            StatusBadge(payment.status)
        }
    }
}

@Composable
private fun StatusBadge(status: String) {
    val color = when (status) {
        "completed" -> Color(0xFF10B981)
        "pending" -> Color(0xFFF59E0B)
        "failed" -> Color(0xFFEF4444)
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Text(
        text = status.replaceFirstChar { it.uppercase() },
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.Medium,
        color = color
    )
}

@Composable
private fun EmptyStateView(
    icon: ImageVector,
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
            Surface(
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surfaceVariant,
                modifier = Modifier.size(64.dp)
            ) {
                Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                    Icon(
                        icon,
                        contentDescription = null,
                        modifier = Modifier.size(32.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            Spacer(Modifier.height(16.dp))
            Text(
                title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold
            )
            Spacer(Modifier.height(4.dp))
            Text(
                description,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center
            )
        }
    }
}
