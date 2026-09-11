package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Restore
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
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.util.ClipboardUtils
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
    var isRestoring by remember { mutableStateOf(false) }
    var showRestoreForceCloseConfirm by remember { mutableStateOf(false) }
    var restoreGuardUnavailable by remember { mutableStateOf(false) }

    /** Wipe + restart the node with the entered seed (post restore-guard). */
    fun performRestore(input: String) {
        isRestoring = true
        restoreError = null
        scope.launch(Dispatchers.IO) {
            try {
                appState.nodeService.stop()
                appState.nodeService.start(Constants.LDK_NETWORK, Constants.PRIMARY_CHAIN_URL, input)
                withContext(Dispatchers.Main) {
                    isRestoring = false
                    showRestore = false
                    restoreMnemonic = ""
                    restoreError = null
                    appState.refreshBalances()
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    isRestoring = false
                    restoreError = e.message ?: "Restore failed"
                }
            }
        }
    }

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
                val wordList = words.split(" ")
                val columns = 3
                val rows = (wordList.size + columns - 1) / columns
                for (row in 0 until rows) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        for (col in 0 until columns) {
                            val index = row * columns + col
                            if (index < wordList.size) {
                                Surface(
                                    shape = RoundedCornerShape(8.dp),
                                    color = MaterialTheme.colorScheme.surfaceVariant,
                                    modifier = Modifier.weight(1f)
                                ) {
                                    Row(
                                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
                                        verticalAlignment = Alignment.CenterVertically
                                    ) {
                                        Text(
                                            "${index + 1}.",
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            modifier = Modifier.width(24.dp)
                                        )
                                        Text(
                                            wordList[index],
                                            fontFamily = FontFamily.Monospace,
                                            style = MaterialTheme.typography.bodyMedium
                                        )
                                    }
                                }
                            } else {
                                Spacer(Modifier.weight(1f))
                            }
                        }
                    }
                    if (row < rows - 1) Spacer(Modifier.height(6.dp))
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
                if (!isRestoring) {
                    showRestore = false
                    restoreMnemonic = ""
                    restoreError = null
                }
            },
            containerColor = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            title = {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Icon(
                        imageVector = Icons.Default.Restore,
                        contentDescription = "Restore",
                        tint = Color(0xFFF59E0B),
                        modifier = Modifier.size(48.dp)
                    )
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Restore from Seed",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold
                    )
                }
            },
            text = {
                Column {
                    Text(
                        "Enter your 12 or 24-word seed phrase to restore a wallet.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                        modifier = Modifier.fillMaxWidth()
                    )
                    Spacer(Modifier.height(16.dp))
                    OutlinedTextField(
                        value = restoreMnemonic,
                        onValueChange = { restoreMnemonic = it },
                        label = { Text("word1 word2 word3 ...") },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3,
                        enabled = !isRestoring,
                        textStyle = MaterialTheme.typography.bodyMedium.copy(
                            fontFamily = FontFamily.Monospace
                        ),
                        shape = RoundedCornerShape(12.dp)
                    )
                    if (restoreError != null) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            restoreError!!,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error
                        )
                    }
                    if (isRestoring) {
                        Spacer(Modifier.height(16.dp))
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.Center,
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(20.dp),
                                strokeWidth = 2.dp
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(
                                "Restoring wallet...",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
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
                        // Restore guard: a seed-only restore wipes LDK state;
                        // if this seed's node still has an LSP channel it will
                        // be force-closed at the next reestablish. Detect that
                        // and require explicit opt-in; fail-warn if the LSP is
                        // unreachable or derivation fails.
                        isRestoring = true
                        restoreError = null
                        scope.launch(Dispatchers.IO) {
                            val nodeId = NodeService.deriveNodeId(context, input)
                            val exists = nodeId?.let { appState.lspChannelExists(it) }
                            if (exists == null) {
                                AuditService.log(
                                    "RESTORE_GUARD_UNAVAILABLE",
                                    mapOf("node_id" to (nodeId ?: "derive_failed"))
                                )
                            }
                            withContext(Dispatchers.Main) {
                                when (exists) {
                                    true -> {
                                        AuditService.log(
                                            "RESTORE_ACTIVE_CHANNEL_DETECTED",
                                            mapOf("node_id" to nodeId)
                                        )
                                        isRestoring = false
                                        restoreGuardUnavailable = false
                                        showRestoreForceCloseConfirm = true
                                    }
                                    // Guard couldn't run — fail-warn: require
                                    // explicit opt-in instead of proceeding.
                                    null -> {
                                        isRestoring = false
                                        restoreGuardUnavailable = true
                                        showRestoreForceCloseConfirm = true
                                    }
                                    false -> performRestore(input)
                                }
                            }
                        }
                    },
                    enabled = restoreMnemonic.trim().isNotEmpty() && !isRestoring
                ) {
                    if (isRestoring) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp
                        )
                    } else {
                        Text("Restore")
                    }
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        showRestore = false
                        restoreMnemonic = ""
                        restoreError = null
                    },
                    enabled = !isRestoring
                ) { Text("Cancel") }
            }
        )
    }

    // Restore guard: explicit opt-in when the seed's node still has an open
    // LSP channel that a seed-only restore would force-close (or when the
    // check could not run).
    if (showRestoreForceCloseConfirm) {
        AlertDialog(
            onDismissRequest = { showRestoreForceCloseConfirm = false },
            containerColor = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            title = {
                Text(
                    if (restoreGuardUnavailable) "Couldn't Verify Channel Status"
                    else "Open Channel Detected"
                )
            },
            text = {
                Text(
                    if (restoreGuardUnavailable) {
                        "The server couldn't be reached to check whether this wallet " +
                            "still has an open Lightning channel. If it does, restoring " +
                            "from seed alone will force-close it on-chain. Continue only " +
                            "if you're sure, or try again with a network connection."
                    } else {
                        "This wallet still has an open Lightning channel with the LSP. " +
                            "Restoring from seed alone cannot restore the channel and it will " +
                            "be force-closed on-chain; funds return after a timelock. Only " +
                            "continue if this is your only way back into the wallet."
                    }
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    showRestoreForceCloseConfirm = false
                    performRestore(restoreMnemonic.trim())
                }) {
                    Text(
                        if (restoreGuardUnavailable) "Continue Anyway" else "Restore Anyway",
                        color = MaterialTheme.colorScheme.error
                    )
                }
            },
            dismissButton = {
                TextButton(onClick = {
                    showRestoreForceCloseConfirm = false
                }) { Text("Cancel") }
            }
        )
    }
}
