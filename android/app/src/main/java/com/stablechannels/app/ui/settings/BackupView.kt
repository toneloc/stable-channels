package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
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
import androidx.fragment.app.FragmentActivity
import com.stablechannels.app.AppState
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.util.ClipboardUtils
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.lightningdevkit.ldknode.Network

@Composable
fun BackupView(appState: AppState) {
    val clipboardManager = LocalClipboardManager.current
    val context = LocalContext.current
    val activity = LocalContext.current as? FragmentActivity
    val scope = rememberCoroutineScope()

    var showSeedWords by remember { mutableStateOf(false) }
    var seedAuthError by remember { mutableStateOf<String?>(null) }
    var showRestore by remember { mutableStateOf(false) }
    var restoreMnemonic by remember { mutableStateOf("") }
    var restoreError by remember { mutableStateOf<String?>(null) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp)
    ) {
        // Show/hide seed words
        Button(
            onClick = {
                if (showSeedWords) {
                    // Hiding seed words — no auth needed
                    showSeedWords = false
                    seedAuthError = null
                } else {
                    val requireAuth = com.stablechannels.app.services.AppAccessPreferencesManager.shouldRequireAuthForSeedPhrase(context)
                    if (requireAuth) {
                        // Showing seed words — require biometric auth
                        scope.launch {
                            if (activity != null) {
                                val authResult = BiometricService.authenticate(activity, "View seed phrase")
                                if (authResult == BiometricService.AuthResult.SUCCESS) {
                                    showSeedWords = true
                                    seedAuthError = null
                                } else {
                                    seedAuthError = "Authentication required"
                                }
                            } else {
                                seedAuthError = "Authentication required"
                            }
                        }
                    } else {
                        showSeedWords = true
                        seedAuthError = null
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(
                containerColor = Color(0xFF10B981),
                contentColor = Color.White
            )
        ) {
            Text(if (showSeedWords) "Hide Seed Words" else "Backup Seed Words")
        }

        seedAuthError?.let {
            Spacer(Modifier.height(4.dp))
            Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        if (showSeedWords) {
            val words = appState.nodeService.savedMnemonic
            if (!words.isNullOrEmpty()) {
                Spacer(Modifier.height(12.dp))
                Text(
                    "Write these words down on paper and store them in a safe place. Never share them. Anyone with these words can access your funds.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Color(0xFFD97706)
                )
                Spacer(Modifier.height(12.dp))
                words.split(" ").forEachIndexed { index, word ->
                    Text(
                        "${index + 1}. $word",
                        fontFamily = FontFamily.Monospace,
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
                Spacer(Modifier.height(12.dp))
                var copied by remember { mutableStateOf(false) }
                var showClipboardWarning by remember { mutableStateOf(false) }
                OutlinedButton(
                    onClick = {
                        showClipboardWarning = true
                    },
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.outlinedButtonColors(
                        contentColor = Color(0xFF3B82F6)
                    ),
                    border = androidx.compose.foundation.BorderStroke(1.dp, Color(0xFF3B82F6))
                ) {
                    Text(if (copied) "Copied" else "Copy Seed Words")
                }

                // Clipboard security confirmation dialog
                if (showClipboardWarning) {
                    AlertDialog(
                        onDismissRequest = { showClipboardWarning = false },
                        containerColor = MaterialTheme.colorScheme.surface,
                        tonalElevation = 3.dp,
                        shape = RoundedCornerShape(20.dp),
                        icon = {
                            Surface(
                                shape = RoundedCornerShape(12.dp),
                                color = Color(0xFFF59E0B).copy(alpha = 0.12f),
                                modifier = Modifier.size(48.dp)
                            ) {
                                Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                                    Text("⚠️", style = MaterialTheme.typography.headlineSmall)
                                }
                            }
                        },
                        title = {
                            Text(
                                "Copy Seed Phrase?",
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.SemiBold
                            )
                        },
                        text = {
                            Text(
                                "Clipboard contents may be readable by other apps. The clipboard will be cleared after 60 seconds.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        },
                        confirmButton = {
                            Button(
                                onClick = {
                                    showClipboardWarning = false
                                    ClipboardUtils.copySensitive(context, "Seed Phrase", words)
                                    copied = true
                                },
                                colors = ButtonDefaults.buttonColors(
                                    containerColor = Color(0xFF10B981),
                                    contentColor = Color.White
                                )
                            ) { Text("Copy") }
                        },
                        dismissButton = {
                            OutlinedButton(
                                onClick = { showClipboardWarning = false }
                            ) { Text("Cancel") }
                        }
                    )
                }
            } else {
                Spacer(Modifier.height(8.dp))
                Text(
                    "Seed phrase not available for this wallet.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

        Spacer(Modifier.height(16.dp))

        // Restore from seed
        OutlinedButton(
            onClick = { showRestore = true },
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.outlinedButtonColors(
                contentColor = Color(0xFF3B82F6)
            ),
            border = androidx.compose.foundation.BorderStroke(1.dp, Color(0xFF3B82F6))
        ) {
            Text("Restore from Seed")
        }
    }

    // Restore dialog
    if (showRestore) {
        AlertDialog(
            onDismissRequest = {
                showRestore = false
                restoreMnemonic = ""
                restoreError = null
            },
            title = { Text("Restore from Seed") },
            text = {
                Column {
                    Text(
                        "Enter your 12 or 24-word seed phrase to restore a wallet.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = restoreMnemonic,
                        onValueChange = { restoreMnemonic = it },
                        label = { Text("word1 word2 word3 ...") },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3
                    )
                    if (restoreError != null) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            restoreError!!,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val input = restoreMnemonic.trim()
                        val wordCount = input.split("\\s+".toRegex()).size
                        if (wordCount != 12 && wordCount != 24) {
                            restoreError = "Seed phrase must be 12 or 24 words"
                            return@TextButton
                        }
                        scope.launch(Dispatchers.IO) {
                            try {
                                appState.nodeService.stop()
                                appState.nodeService.start(Network.BITCOIN, Constants.PRIMARY_CHAIN_URL, input)
                                withContext(Dispatchers.Main) {
                                    showRestore = false
                                    restoreMnemonic = ""
                                    restoreError = null
                                    appState.refreshBalances()
                                }
                            } catch (e: Exception) {
                                withContext(Dispatchers.Main) {
                                    restoreError = e.message ?: "Restore failed"
                                }
                            }
                        }
                    },
                    enabled = restoreMnemonic.trim().isNotEmpty()
                ) { Text("Restore") }
            },
            dismissButton = {
                TextButton(onClick = {
                    showRestore = false
                    restoreMnemonic = ""
                    restoreError = null
                }) { Text("Cancel") }
            }
        )
    }
}
