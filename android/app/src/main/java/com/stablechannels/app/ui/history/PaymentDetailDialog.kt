package com.stablechannels.app.ui.history

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted

@Composable
fun PaymentDetailDialog(payment: PaymentRecord, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Payment Details") },
        text = {
            Column {
                DetailRow("Direction", if (payment.isIncoming) "Received" else "Sent")
                val typeLabel = when (payment.paymentType) {
                    "stability" -> "Stability"
                    "splice_in" -> "Splice In"
                    "splice_out" -> "Splice Out"
                    "onchain" -> "On-chain"
                    "channel_close" -> "Channel Close"
                    else -> "Lightning"
                }
                DetailRow("Type", typeLabel)
                DetailRow("Amount", payment.amountSats.satsFormatted())
                payment.amountUSD?.let { DetailRow("USD Value", it.usdFormatted()) }
                payment.btcPrice?.let { DetailRow("BTC Price", it.usdFormatted()) }
                if (payment.feeMsat > 0) DetailRow("Fee", (payment.feeMsat / 1000).satsFormatted())
                DetailRow("Status", payment.status)
                DetailRow("Date", payment.date.toString())
                payment.paymentId?.let { DetailRow("Payment ID", it) }
                payment.txid?.let { DetailRow("TXID", it) }
                payment.address?.let { DetailRow("Address", it) }
                if (payment.confirmations > 0) DetailRow("Confirmations", payment.confirmations.toString())
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        }
    )
}
