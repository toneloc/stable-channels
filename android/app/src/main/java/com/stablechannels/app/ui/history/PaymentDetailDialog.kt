package com.stablechannels.app.ui.history

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import com.stablechannels.app.util.shortString
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.isSystemInDarkTheme
import com.stablechannels.app.util.Constants
import androidx.compose.ui.platform.LocalContext
import android.content.Intent
import android.net.Uri

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PaymentDetailBottomSheet(payment: PaymentRecord, currentPrice: Double = 0.0, onDismiss: () -> Unit) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
        containerColor = if (isSystemInDarkTheme()) Color.Black else Color.White
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
                    text = "Payment Details",
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
                    DetailRow("Direction", if (payment.isIncoming) "Received" else "Sent")
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    
                    val typeLabel = when (payment.paymentType) {
                        "stability" -> "Stability"
                        "splice_in" -> "Splice In"
                        "splice_out" -> "Splice Out"
                        "onchain" -> "Onchain"
                        "channel_close" -> "Channel Close"
                        else -> "Lightning"
                    }
                    DetailRow("Type", typeLabel)
                    
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    val usdVal = payment.amountUSD ?: run {
                        val price = payment.btcPrice?.takeIf { it > 0.0 } ?: currentPrice.takeIf { it > 0.0 }
                        price?.let { (payment.amountSats.toDouble() / Constants.SATS_IN_BTC) * it }
                    }
                    val amountStr = usdVal?.usdFormatted() ?: "${payment.amountSats.satsFormatted()} sats"
                    DetailRow("Amount", amountStr)
                    
                    payment.btcPrice?.let {
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        DetailRow("BTC Price", it.usdFormatted())
                    }
                    
                    if (payment.feeMsat > 0) {
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        DetailRow("Fee", "${payment.feeMsat / 1000} sats")
                    }
                    
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("Status", payment.status.replaceFirstChar { it.uppercase() })
                    
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    DetailRow("Date", payment.date.shortString())
                    
                    payment.paymentId?.let { pid ->
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        val displayPid = if (pid.length > 16) pid.take(8) + "..." + pid.takeLast(8) else pid
                        CopyableDetailRow("Payment ID", displayPid, pid)
                    }
                    
                    payment.txid?.let { txid ->
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        val displayTxid = if (txid.length > 16) txid.take(8) + "..." + txid.takeLast(8) else txid
                        CopyableDetailRow("TXID", displayTxid, txid)
                        val context = LocalContext.current
                        val onchainTypes = setOf("channel_close", "onchain", "splice_in", "splice_out")
                        if (payment.paymentType in onchainTypes) {
                            TextButton(
                                onClick = {
                                    val cleanTxid = txid.substringBefore(":")
                                    val intent = Intent(Intent.ACTION_VIEW, Uri.parse("https://mempool.space/tx/$cleanTxid"))
                                    context.startActivity(intent)
                                },
                                contentPadding = PaddingValues(0.dp)
                            ) {
                                Text("View on explorer", style = MaterialTheme.typography.labelSmall)
                            }
                        }
                    }
                    
                    payment.address?.let { addr ->
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        val displayAddr = if (addr.length > 16) addr.take(8) + "..." + addr.takeLast(8) else addr
                        CopyableDetailRow("Address", displayAddr, addr)
                    }
                    
                    if (payment.confirmations > 0) {
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        DetailRow("Confirmations", payment.confirmations.toString())
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
