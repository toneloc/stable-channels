package com.stablechannels.app.ui.history

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.stablechannels.app.models.TradeRecord
import com.stablechannels.app.util.usdFormatted
import java.util.Locale

@Composable
fun TradeDetailDialog(trade: TradeRecord, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Trade Details") },
        text = {
            Column {
                DetailRow("Action", if (trade.action == "buy") "Buy BTC" else "Sell BTC")
                DetailRow("Amount", trade.amountUSD.usdFormatted())
                DetailRow("BTC Amount", String.format(Locale.US, "%.8f BTC", trade.amountBTC))
                DetailRow("BTC Price", trade.btcPrice.usdFormatted())
                DetailRow("Fee", trade.feeUSD.usdFormatted())
                DetailRow("Status", trade.status)
                DetailRow("Date", trade.date.toString())
                trade.paymentId?.let { DetailRow("Payment ID", it) }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        }
    )
}

@Composable
fun DetailRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}
