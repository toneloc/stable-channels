package com.stablechannels.app.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp

@Composable
fun LogsView() {
    val context = LocalContext.current

    Column(Modifier.padding(16.dp)) {
        Text("Logs & Diagnostics", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        Text(
            "Save app logs to a file for debugging and support.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Spacer(Modifier.height(16.dp))
        
        Card(
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f)),
            shape = androidx.compose.foundation.shape.RoundedCornerShape(16.dp)
        ) {
            Column(Modifier.padding(vertical = 8.dp)) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable { com.stablechannels.app.services.LogExporter.shareLogs(context) }
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(Icons.Outlined.Share, contentDescription = null, tint = androidx.compose.ui.graphics.Color(0xFF4CAF50))
                    Spacer(Modifier.width(16.dp))
                    Text("Share the logs", color = androidx.compose.ui.graphics.Color(0xFF4CAF50))
                }
                HorizontalDivider(
                    Modifier.padding(horizontal = 16.dp),
                    thickness = 0.5.dp,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.1f)
                )
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable { com.stablechannels.app.services.LogExporter.downloadLogs(context) }
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(Icons.Outlined.Info, contentDescription = null, tint = androidx.compose.ui.graphics.Color(0xFF4CAF50))
                    Spacer(Modifier.width(16.dp))
                    Text("Download logs", color = androidx.compose.ui.graphics.Color(0xFF4CAF50))
                }
            }
        }
    }
}
