package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.*
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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.fragment.app.FragmentActivity
import com.stablechannels.app.AppState
import com.stablechannels.app.services.AppAccessPreferencesManager
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.ui.components.InkButton
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.ui.settings.seed.RevealSecretPhraseDialog
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
    var restoreAuthError by remember { mutableStateOf<String?>(null) }
    var restoreMnemonic by remember { mutableStateOf("") }
    var restoreError by remember { mutableStateOf<String?>(null) }
    var isRestoring by remember { mutableStateOf(false) }
    var showRestoreForceCloseConfirm by remember { mutableStateOf(false) }
    var restoreGuardUnavailable by remember { mutableStateOf(false) }

    /**
     * Wipe + restart the node with the entered seed (post restore-guard), then look on chain for
     * the LSP's close of an old channel (#364).
     */
    fun performRestore(input: String, channelMayExist: Boolean) {
        isRestoring = true
        restoreError = null
        scope.launch(Dispatchers.IO) {
            try {
                appState.startRestoredNode(input, channelMayExist)
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

    Column(modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        InkButton(
            onClick = {
                val requireAuth =
                    AppAccessPreferencesManager.shouldRequireAuthForSeedPhrase(context)
                if (requireAuth) {
                    scope.launch {
                        if (activity != null) {
                            val authResult =
                                BiometricService.authenticate(activity, "View seed phrase")
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
            },
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = Color(0xFF10B981),
                    contentColor = Color.White,
                ),
        ) {
            Text("Backup Seed Words")
        }

        seedAuthError?.let {
            Spacer(Modifier.height(4.dp))
            Text(
                it,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodySmall,
            )
        }

        if (showSeedWords) {
            if (!appState.nodeService.savedMnemonic.isNullOrEmpty()) {
                // The quiz reads the phrase itself, only once it is passed.
                RevealSecretPhraseDialog(
                    mnemonicProvider = { appState.nodeService.savedMnemonic },
                    onDismiss = { showSeedWords = false },
                )
            } else {
                Spacer(Modifier.height(8.dp))
                Text(
                    "Seed phrase not available for this wallet.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Spacer(Modifier.height(16.dp))

        // Restore from seed — wipes this wallet, so it needs the same auth as viewing the seed
        InkButton(
            onClick = {
                restoreAuthError = null
                if (!AppAccessPreferencesManager.shouldRequireAuthForSeedPhrase(context)) {
                    showRestore = true
                } else if (activity == null) {
                    restoreAuthError = "Authentication required"
                } else {
                    scope.launch {
                        val authResult =
                            BiometricService.authenticate(activity, "Restore from seed")
                        if (authResult == BiometricService.AuthResult.SUCCESS) {
                            showRestore = true
                        } else {
                            restoreAuthError = "Authentication required"
                        }
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Restore from Seed")
        }

        restoreAuthError?.let {
            Spacer(Modifier.height(4.dp))
            Text(
                it,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodySmall,
            )
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
            // The typed seed stays out of screenshots and the recents thumbnail.
            properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
            containerColor = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            title = {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(
                        imageVector = Icons.Default.Restore,
                        contentDescription = "Restore",
                        tint = Color(0xFFF59E0B),
                        modifier = Modifier.size(48.dp),
                    )
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Restore from Seed",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
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
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(Modifier.height(16.dp))
                    OutlinedTextField(
                        value = restoreMnemonic,
                        onValueChange = { restoreMnemonic = it },
                        label = { Text("word1 word2 word3 ...") },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3,
                        enabled = !isRestoring,
                        textStyle =
                            MaterialTheme.typography.bodyMedium.copy(
                                fontFamily = FontFamily.Monospace
                            ),
                        shape = RoundedCornerShape(12.dp),
                    )
                    if (restoreError != null) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            restoreError!!,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                    if (isRestoring) {
                        Spacer(Modifier.height(16.dp))
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.Center,
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(20.dp),
                                strokeWidth = 2.dp,
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(
                                "Restoring wallet...",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            },
            confirmButton = {
                NeutralTextButton(
                    onClick = {
                        val input = restoreMnemonic.trim()
                        val wordCount = input.split("\\s+".toRegex()).size
                        if (wordCount != 12 && wordCount != 24) {
                            restoreError = "Seed phrase must be 12 or 24 words"
                            return@NeutralTextButton
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
                                    mapOf("node_id" to (nodeId ?: "derive_failed")),
                                )
                            }
                            withContext(Dispatchers.Main) {
                                when (exists) {
                                    true -> {
                                        AuditService.log(
                                            "RESTORE_ACTIVE_CHANNEL_DETECTED",
                                            mapOf("node_id" to nodeId),
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
                                    false -> performRestore(input, channelMayExist = false)
                                }
                            }
                        }
                    },
                    enabled = restoreMnemonic.trim().isNotEmpty() && !isRestoring,
                ) {
                    if (isRestoring) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp,
                        )
                    } else {
                        Text("Restore")
                    }
                }
            },
            dismissButton = {
                NeutralTextButton(
                    onClick = {
                        showRestore = false
                        restoreMnemonic = ""
                        restoreError = null
                    },
                    enabled = !isRestoring,
                ) {
                    Text("Cancel")
                }
            },
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
                            "be force-closed on-chain. If the channel was open on this phone, the " +
                            "app then moves your funds to your onchain wallet automatically. Only " +
                            "continue if this is your only way back into the wallet."
                    }
                )
            },
            confirmButton = {
                NeutralTextButton(
                    onClick = {
                        showRestoreForceCloseConfirm = false
                        performRestore(restoreMnemonic.trim(), channelMayExist = true)
                    }
                ) {
                    Text(
                        if (restoreGuardUnavailable) "Continue Anyway" else "Restore Anyway",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            },
            dismissButton = {
                NeutralTextButton(
                    onClick = {
                        showRestoreForceCloseConfirm = false
                    }
                ) {
                    Text("Cancel")
                }
            },
        )
    }
}
