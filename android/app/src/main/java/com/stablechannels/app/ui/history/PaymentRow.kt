package com.stablechannels.app.ui.history

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowCircleDown
import androidx.compose.material.icons.filled.ArrowCircleUp
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.relativeString
import com.stablechannels.app.util.usdFormatted

@Composable
internal fun PaymentRow(
    payment: PaymentRecord,
    currentPrice: Double,
    compact: Boolean = false,
    onClick: () -> Unit,
) {
    val isIncoming = payment.isIncoming
    val icon = if (isIncoming) Icons.Default.ArrowCircleDown else Icons.Default.ArrowCircleUp
    val iconColor = if (isIncoming) ReceivedGreen else SentBlue
    val typeLabel =
        when (payment.paymentType) {
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
        modifier =
            Modifier.fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = if (compact) 8.dp else 12.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Icon with colored background
        Surface(
            shape = RoundedCornerShape(10.dp),
            color = iconColor.copy(alpha = 0.12f),
            modifier = Modifier.size(if (compact) 32.dp else 40.dp),
        ) {
            Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                Icon(
                    icon,
                    contentDescription = null,
                    tint = iconColor,
                    modifier = Modifier.size(if (compact) 18.dp else 22.dp),
                )
            }
        }

        Spacer(Modifier.width(if (compact) 10.dp else 12.dp))

        // Title + type + time
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = if (isIncoming) "Received" else "Sent",
                style =
                    if (compact) MaterialTheme.typography.bodyMedium
                    else MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = "$typeLabel · ${payment.date.relativeString()}",
                style =
                    if (compact) MaterialTheme.typography.labelSmall
                    else MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // Amount + status
        Column(horizontalAlignment = Alignment.End) {
            Text(
                text = payment.signedAmountText(currentPrice),
                style =
                    if (compact) MaterialTheme.typography.bodyMedium
                    else MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
                color = payment.amountColor(),
            )
            val statusLabel = payment.historyStatusLabel()
            val statusColor = payment.historyStatusColor()
            StatusBadge(statusLabel, statusColor)
        }
    }
}

@Composable
internal fun StatusBadge(status: String, color: Color? = null) {
    val resolvedColor =
        color
            ?: when (status.lowercase()) {
                "completed",
                "accepted" -> Color(0xFF10B981)
                "pending",
                "prepared",
                "sent",
                "fee_paid",
                "uncertain" -> Color(0xFFF59E0B)
                "failed",
                "send_failed",
                "rejected" -> Color(0xFFEF4444)
                else -> MaterialTheme.colorScheme.onSurfaceVariant
            }
    Text(
        text =
            when (status) {
                "send_failed" -> "Failed"
                "fee_paid" -> "Awaiting result"
                "uncertain" -> "Result delayed"
                else -> status
            },
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.Medium,
        color = resolvedColor,
    )
}

private fun PaymentRecord.shouldShowConfirmationProgress(): Boolean {
    val onchainTypes = setOf("onchain", "channel_close", "splice_in", "splice_out")
    val hasProgressSignal = status == "pending" || confirmations > 0
    return paymentType in onchainTypes && hasProgressSignal
}

private fun PaymentRecord.historyStatusLabel(): String {
    if (!shouldShowConfirmationProgress()) {
        return status.replaceFirstChar { it.uppercase() }
    }
    val required = requiredConfirmationsForDisplay()
    return if (confirmations >= required) {
        "Confirmed"
    } else {
        "${confirmations}/${required} confirmed"
    }
}

private fun PaymentRecord.historyStatusColor(): Color {
    if (!shouldShowConfirmationProgress()) {
        return when (status) {
            "completed" -> Color(0xFF10B981)
            "pending" -> Color(0xFFF59E0B)
            "failed" -> Color(0xFFEF4444)
            else -> Color(0xFF6B7280)
        }
    }
    return when {
        confirmations >= requiredConfirmationsForDisplay() -> Color(0xFF10B981)
        confirmations > 0 -> Color(0xFF3B82F6)
        else -> Color(0xFFF59E0B)
    }
}

private fun PaymentRecord.requiredConfirmationsForDisplay(): Int {
    return AppState.requiredConfirmationsForType(paymentType)
}

internal val ReceivedGreen = Color(0xFF10B981)
internal val SentRed = Color(0xFFEF4444)
private val SentBlue = Color(0xFF3B82F6)

internal fun PaymentRecord.signedAmountText(currentPrice: Double): String {
    val displayUsd =
        amountUSD
            ?: (btcPrice?.takeIf { it > 0.0 } ?: currentPrice.takeIf { it > 0.0 })?.let {
                (amountSats.toDouble() / Constants.SATS_IN_BTC) * it
            }
    val amountText = displayUsd?.usdFormatted() ?: "${amountSats.btcSpacedFormatted()} BTC"
    return (if (isIncoming) "+" else "-") + amountText
}

@Composable
private fun PaymentRecord.amountColor(): Color =
    when {
        status == "failed" -> MaterialTheme.colorScheme.onSurfaceVariant
        isIncoming -> ReceivedGreen
        else -> SentRed
    }
