package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.util.LspPreferencesManager
import kotlinx.coroutines.launch

@Composable
fun LspSettingsView(appState: AppState) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val clipboardManager = LocalClipboardManager.current

    // Recompute whenever a switch/reset completes.
    var refreshKey by remember { mutableStateOf(0) }
    val activePubkey = remember(refreshKey) { LspPreferencesManager.getLspPubkey(context) }
    val activeAddress = remember(refreshKey) { LspPreferencesManager.getLspAddress(context) }
    val isCustom = remember(refreshKey) { LspPreferencesManager.hasCustomLsp(context) }

    val hasActiveChannels = appState.nodeService.channels.isNotEmpty()

    var showSwitchDialog by remember { mutableStateOf(false) }
    var isBusy by remember { mutableStateOf(false) }
    var resultMessage by remember { mutableStateOf<String?>(null) }
    var copiedNodeId by remember { mutableStateOf(false) }

    fun finish(error: String?) {
        isBusy = false
        resultMessage = error ?: "LSP updated successfully."
        refreshKey++
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Connection",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(bottom = 12.dp)
        )

        Surface(
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("LSP Address", style = MaterialTheme.typography.bodyLarge)
                    if (isCustom) {
                        Text(
                            text = "Custom",
                            style = MaterialTheme.typography.labelSmall,
                            color = Color(0xFF3B82F6)
                        )
                    }
                }
                Spacer(Modifier.height(4.dp))
                Text(
                    text = activeAddress,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )

                Spacer(Modifier.height(16.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Node ID", style = MaterialTheme.typography.bodyLarge)
                    TextButton(onClick = {
                        clipboardManager.setText(AnnotatedString(activePubkey))
                        copiedNodeId = true
                    }) {
                        Text(if (copiedNodeId) "Copied ✓" else "Copy")
                    }
                }
                Text(
                    text = activePubkey,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

        Spacer(Modifier.height(24.dp))

        if (hasActiveChannels) {
            Surface(
                shape = MaterialTheme.shapes.medium,
                color = Color(0xFFF59E0B).copy(alpha = 0.12f),
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = "Your LSP cannot be changed while channels are active. Close all channels before switching.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color(0xFFF59E0B),
                    modifier = Modifier.padding(16.dp)
                )
            }
        } else {
            Button(
                onClick = { showSwitchDialog = true },
                enabled = !isBusy,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Switch LSP")
            }

            Spacer(Modifier.height(12.dp))

            OutlinedButton(
                onClick = {
                    isBusy = true
                    resultMessage = null
                    scope.launch {
                        appState.resetLspToDefault { error -> finish(error) }
                    }
                },
                enabled = !isBusy && isCustom,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Reset")
            }
        }

        if (isBusy) {
            Spacer(Modifier.height(16.dp))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                Text("Restarting node...", style = MaterialTheme.typography.bodyMedium)
            }
        }

        resultMessage?.let { message ->
            Spacer(Modifier.height(16.dp))
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = if (message.startsWith("LSP updated")) Color(0xFF10B981) else Color(0xFFEF4444)
            )
        }
    }

    if (showSwitchDialog) {
        SwitchLspDialog(
            onDismiss = { showSwitchDialog = false },
            onSubmit = { pubkey, address ->
                showSwitchDialog = false
                isBusy = true
                resultMessage = null
                scope.launch {
                    appState.switchLsp(pubkey, address) { error -> finish(error) }
                }
            }
        )
    }
}

@Composable
private fun SwitchLspDialog(
    onDismiss: () -> Unit,
    onSubmit: (pubkey: String, address: String) -> Unit
) {
    var pubkey by remember { mutableStateOf("") }
    var address by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }

    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        title = { Text("Switch LSP") },
        text = {
            Column {
                Text(
                    text = "Enter the details of a compatible Lightning Service Provider. Your node will restart to apply the new configuration.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(16.dp))
                OutlinedTextField(
                    value = pubkey,
                    onValueChange = { pubkey = it; error = null },
                    label = { Text("Pubkey") },
                    placeholder = { Text("02... or 03... (66 hex chars)") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it; error = null },
                    label = { Text("Address") },
                    placeholder = { Text("domain.com:9735") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                error?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                if (!LspPreferencesManager.isValidPubkey(pubkey)) {
                    error = "Pubkey must start with 02 or 03 and be exactly 66 hex characters."
                    return@TextButton
                }
                if (!LspPreferencesManager.isValidAddress(address)) {
                    error = "Address must be in host:port format (e.g. domain.com:9735)."
                    return@TextButton
                }
                onSubmit(pubkey.trim(), address.trim())
            }) { Text("Save") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        }
    )
}
