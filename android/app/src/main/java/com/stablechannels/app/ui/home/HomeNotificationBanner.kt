package com.stablechannels.app.ui.home

import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
fun HomeNotificationBanner(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    Card(
        onClick = {
            val intent =
                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
                    putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                }
            context.startActivity(intent)
        },
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.error),
        modifier = modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(
                Icons.Default.Warning,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onError,
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    "Notifications Disabled",
                    color = MaterialTheme.colorScheme.onError,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 14.sp,
                )
                Text(
                    "Enable notifications for stability payments",
                    color = MaterialTheme.colorScheme.onError.copy(alpha = 0.9f),
                    fontSize = 12.sp,
                )
            }
            Icon(
                imageVector = Icons.Default.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onError.copy(alpha = 0.7f),
                modifier = Modifier.size(16.dp),
            )
        }
    }
}
