package com.stablechannels.app.ui.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.SouthEast
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

val SendBlue = Color(0xFF59A6FF)
val ReceiveGreen = Color(0xFF40D98C)
val BuyAmber = Color(0xFFFF9E40)
val SellPlum = Color(0xFFC78CFF)

@Composable
fun HomeActionButtons(
    hasReadyChannel: Boolean,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    onBuy: () -> Unit,
    onSell: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            ActionButton(
                title = "Send",
                icon = Icons.Default.ArrowUpward,
                color = SendBlue,
                modifier = Modifier.weight(1f),
                onClick = onSend,
            )
            ActionButton(
                title = "Receive",
                icon = Icons.Default.ArrowDownward,
                color = ReceiveGreen,
                pulse = !hasReadyChannel,
                modifier = Modifier.weight(1f),
                onClick = onReceive,
            )
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            ActionButton(
                title = "USD → BTC",
                icon = Icons.Default.NorthEast,
                color = BuyAmber,
                enabled = hasReadyChannel,
                modifier = Modifier.weight(1f),
                onClick = onBuy,
            )
            ActionButton(
                title = "BTC → USD",
                icon = Icons.Default.SouthEast,
                color = SellPlum,
                enabled = hasReadyChannel,
                modifier = Modifier.weight(1f),
                onClick = onSell,
            )
        }
    }
}
