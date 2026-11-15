package com.stablechannels.app.ui.home

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.util.openInAppBrowser

/**
 * Shown on Home while the app's own close of a channel is slow to confirm (#456). The app keeps
 * sending the close itself, so the one thing left for the user is paying a higher fee.
 */
@Composable
fun OwnForceCloseCard(closeTxid: String) {
    var showSpeedUp by remember { mutableStateOf(false) }
    val context = LocalContext.current
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        elevation = CardDefaults.cardElevation(defaultElevation = 0.dp),
    ) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp)) {
            Text(
                "Close is slow to confirm",
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                "The app keeps sending it. You can pay a higher fee to confirm it sooner.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            NeutralTextButton(
                onClick = { showSpeedUp = true },
                contentPadding = PaddingValues(0.dp),
            ) {
                Text("Speed up")
            }
        }
    }
    if (showSpeedUp) {
        AlertDialog(
            onDismissRequest = { showSpeedUp = false },
            containerColor = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            title = { Text("Speed up the close") },
            text = {
                Text(
                    "mempool.space can get your close confirmed sooner for an extra fee. On the page that opens, tap Accelerate and follow the steps."
                )
            },
            confirmButton = {
                NeutralTextButton(
                    onClick = {
                        showSpeedUp = false
                        context.openInAppBrowser("https://mempool.space/tx/$closeTxid")
                    }
                ) {
                    Text("Open mempool.space")
                }
            },
            dismissButton = {
                NeutralTextButton(onClick = { showSpeedUp = false }) { Text("Not now") }
            },
        )
    }
}
