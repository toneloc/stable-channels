package com.stablechannels.app.ui.history

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.models.TradeRecord
import com.stablechannels.app.util.usdFormatted
import com.stablechannels.app.util.shortString
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.Constants
import androidx.compose.foundation.isSystemInDarkTheme
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OrderDetailBottomSheet(trade: TradeRecord, onDismiss: () -> Unit) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
        containerColor = if (isSystemInDarkTheme()) Color.Black else Color.White,
        contentWindowInsets = @Composable { WindowInsets(0, 0, 0, 0) }
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(bottom = 24.dp, start = 24.dp, end = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            // Header Row (Item 32: cancel button in bottomsheet, Item 12: title at center)
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
                        }
                    ),
                    shape = RoundedCornerShape(20.dp),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                ) {
                    Text("Cancel", style = MaterialTheme.typography.bodyMedium)
                }
                Text(
                    text = "Order Details",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.align(Alignment.Center)
                )
            }

            Spacer(Modifier.height(16.dp))

            // Details rows in a nice rounded block matching iOS list look
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = if (isSystemInDarkTheme()) {
                        MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
                    } else {
                        Color(0xFFF2F2F7)
                    }
                ),
                shape = RoundedCornerShape(12.dp),
                modifier = Modifier.fillMaxWidth()
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    DetailRow("Action", if (trade.action == "buy") "USD → BTC" else "BTC → USD")
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("Amount", trade.amountUSD.usdFormatted())
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("BTC Amount", Math.round(trade.amountBTC * Constants.SATS_IN_BTC).btcSpacedFormatted())
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("BTC Price", trade.btcPrice.usdFormatted())
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("Fee", trade.feeUSD.usdFormatted())
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("Status", trade.status.replaceFirstChar { it.uppercase() })
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("Date", trade.date.shortString())
                    trade.paymentId?.let { pid ->
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        val displayPid = if (pid.length > 16) pid.take(8) + "..." + pid.takeLast(8) else pid
                        CopyableDetailRow("Payment ID", displayPid, pid)
                    }
                }
            }

            Spacer(Modifier.height(24.dp))

            // Action Button (Item 33: button should be below and center where thumb is)
            Button(
                onClick = onDismiss,
                modifier = Modifier.fillMaxWidth(0.6f),
                shape = RoundedCornerShape(12.dp)
            ) {
                Text("Done", fontWeight = FontWeight.Bold)
            }
        }
    }
}

@Composable
fun DetailRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
    }
}

@Composable
fun CopyableDetailRow(label: String, value: String, fullValue: String) {
    val clipboardManager = LocalClipboardManager.current
    var copied by remember { mutableStateOf(false) }

    LaunchedEffect(copied) {
        if (copied) {
            kotlinx.coroutines.delay(2000)
            copied = false
        }
    }

    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.End
        ) {
            Text(
                text = value,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                maxLines = 1
            )
            Spacer(Modifier.width(6.dp))
            IconButton(
                onClick = {
                    clipboardManager.setText(AnnotatedString(fullValue))
                    copied = true
                },
                modifier = Modifier.size(24.dp)
            ) {
                Icon(
                    imageVector = if (copied) Icons.Default.Check else Icons.Default.ContentCopy,
                    contentDescription = "Copy",
                    modifier = Modifier.size(14.dp),
                    tint = if (copied) Color(0xFF10B981) else MaterialTheme.colorScheme.primary
                )
            }
        }
    }
}
