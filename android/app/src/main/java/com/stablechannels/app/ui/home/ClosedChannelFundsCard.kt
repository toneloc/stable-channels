package com.stablechannels.app.ui.home

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.services.ClosedChannelFunds

/** Message text for funds held by a closed channel. Pure so it is unit-testable. */
object ClosedChannelFundsMessage {
    fun text(funds: ClosedChannelFunds, btcPrice: Double): String {
        val amount = PendingAmountFormatter.amountText(funds.amountSats, btcPrice)
        val blocksLeft = funds.blocksLeft
        val timing =
            if (blocksLeft != null && blocksLeft > 0)
                "It unlocks in ${ClosedChannelFunds.blocksText(blocksLeft)}, then moves to your onchain wallet automatically."
            else "It moves to your onchain wallet automatically."
        return "$amount is safe on the Bitcoin blockchain. $timing Keep the app installed until it arrives."
    }
}

/** Shown on Home while a closed channel's funds are not in the onchain wallet yet (#358). */
@Composable
fun ClosedChannelFundsCard(funds: ClosedChannelFunds, btcPrice: Double) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        elevation = CardDefaults.cardElevation(defaultElevation = 0.dp),
    ) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Text(
                "Channel closed",
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                ClosedChannelFundsMessage.text(funds, btcPrice),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
