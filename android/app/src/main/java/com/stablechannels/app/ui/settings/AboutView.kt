package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.BuildConfig

@Composable
fun AboutView() {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        // App info card
        Surface(
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                AboutRow("Version", BuildConfig.VERSION_NAME)
                Spacer(Modifier.height(14.dp))
                AboutRow("Network", "Bitcoin", valueColor = Color(0xFFF59E0B))
                Spacer(Modifier.height(14.dp))
                AboutRow("Custody", "Self-custodial", valueColor = Color(0xFF10B981))
            }
        }

        Spacer(Modifier.height(20.dp))

        Text(
            text = "Stable Channels is a self-custodial Bitcoin wallet that maintains a stable USD value using Lightning Network channels. You control your private keys. No third party can access or freeze your funds.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun AboutRow(label: String, value: String, valueColor: Color = Color.Unspecified) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(text = label, style = MaterialTheme.typography.bodyLarge)
        Text(
            text = value,
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.Medium,
            color = if (valueColor != Color.Unspecified) valueColor else MaterialTheme.colorScheme.onSurface
        )
    }
}
