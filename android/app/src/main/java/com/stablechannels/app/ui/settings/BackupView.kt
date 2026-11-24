package com.stablechannels.app.ui.settings

import android.widget.Toast
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.fragment.app.FragmentActivity
import com.stablechannels.app.AppState
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import com.stablechannels.app.domain.seed.SeedAccessPolicy
import com.stablechannels.app.services.AppAccessPreferencesManager
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.ui.settings.seed.ObserveSeedLifecyclePhase
import com.stablechannels.app.ui.settings.seed.RevealSecretPhraseDialog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun BackupView(appState: AppState) {
    val context = LocalContext.current
    val activity = LocalContext.current as? FragmentActivity
    val scope = rememberCoroutineScope()

    var showSeedWords by remember { mutableStateOf(false) }
    val restore = remember { RestoreFlow() }
    // Read after the auth prompt returns, so a reply that lands in background opens nothing.
    var lifecyclePhase by remember { mutableStateOf(AppLifecyclePhase.ACTIVE) }

    /**
     * Wipe + restart the node with the entered seed (post restore-guard), then look on chain for
     * the LSP's close of an old channel (#364).
     */
    fun performRestore(input: String, channelMayExist: Boolean) {
        if (!restore.startApply(input)) return
        scope.launch(Dispatchers.IO) {
            try {
                appState.startRestoredNode(input, channelMayExist)
                withContext(Dispatchers.Main) {
                    restore.onApplied()
                    appState.refreshBalances()
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    restore.onApplyFailed(e.message ?: "Restore failed")
                }
            }
        }
    }

    // The reveal dialog closes itself on background; the restore dialog holds a typed phrase too.
    ObserveSeedLifecyclePhase { phase ->
        lifecyclePhase = phase
        if (SeedAccessPolicy.shouldCloseSeedScreens(phase)) restore.onBackground()
    }

    val gate = remember {
        SeedAccessGate(
            scope = scope,
            requiresAuth = { AppAccessPreferencesManager.shouldRequireAuthForSeedPhrase(context) },
            authenticate = { reason ->
                activity?.let { BiometricService.authenticate(it, reason) }
                    ?: BiometricService.AuthResult.CANCELLED
            },
            phase = { lifecyclePhase },
            onAuthUnavailable = {
                Toast.makeText(context, AUTH_UNAVAILABLE_MESSAGE, Toast.LENGTH_SHORT).show()
            },
        )
    }

    val savedMnemonic by appState.nodeService.savedMnemonicFlow.collectAsState()
    GatedBackupSettingsList(
        canReveal = SeedAccessPolicy.canRevealPhrase(savedMnemonic),
        gate = gate,
        onReveal = { showSeedWords = true },
        onRestore = { restore.open() },
    )

    if (showSeedWords) {
        if (SeedAccessPolicy.canRevealPhrase(savedMnemonic)) {
            // The quiz reads the phrase itself, only once it is passed.
            RevealSecretPhraseDialog(
                mnemonicProvider = { appState.nodeService.savedMnemonic },
                onDismiss = { showSeedWords = false },
            )
        } else {
            // No saved phrase: close at once rather than leave an empty dialog with no way out.
            LaunchedEffect(Unit) { showSeedWords = false }
        }
    }

    // Restore dialog
    if (restore.isOpen) {
        AlertDialog(
            onDismissRequest = { restore.close() },
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
                        value = restore.phrase,
                        onValueChange = { restore.phrase = it },
                        label = { Text("word1 word2 word3 ...") },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3,
                        enabled = !restore.isBusy,
                        textStyle =
                            MaterialTheme.typography.bodyMedium.copy(
                                fontFamily = FontFamily.Monospace
                            ),
                        shape = RoundedCornerShape(12.dp),
                    )
                    restore.error?.let { error ->
                        Spacer(Modifier.height(8.dp))
                        Text(
                            error,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                    if (restore.isBusy) {
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
                        val check = restore.startCheck() ?: return@NeutralTextButton
                        // Restore guard: a seed-only restore wipes LDK state;
                        // if this seed's node still has an LSP channel it will
                        // be force-closed at the next reestablish. Detect that
                        // and require explicit opt-in; fail-warn if the LSP is
                        // unreachable or derivation fails.
                        scope.launch(Dispatchers.IO) {
                            val nodeId = NodeService.deriveNodeId(context, check.phrase)
                            val exists = nodeId?.let { appState.lspChannelExists(it) }
                            if (exists == null) {
                                AuditService.log(
                                    "RESTORE_GUARD_UNAVAILABLE",
                                    mapOf("node_id" to (nodeId ?: "derive_failed")),
                                )
                            } else if (exists) {
                                AuditService.log(
                                    "RESTORE_ACTIVE_CHANNEL_DETECTED",
                                    mapOf("node_id" to nodeId),
                                )
                            }
                            withContext(Dispatchers.Main) {
                                restore.onChecked(check, exists)?.let {
                                    performRestore(it, channelMayExist = false)
                                }
                            }
                        }
                    },
                    enabled = restore.phrase.trim().isNotEmpty() && !restore.isBusy,
                ) {
                    if (restore.isBusy) {
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
                    onClick = { restore.close() },
                    enabled = !restore.isBusy,
                ) {
                    Text("Cancel")
                }
            },
        )
    }

    // Restore guard: explicit opt-in when the seed's node still has an open
    // LSP channel that a seed-only restore would force-close (or when the
    // check could not run).
    restore.warning?.let { warning ->
        val restoreGuardUnavailable = warning == RestoreWarning.UNVERIFIED
        AlertDialog(
            onDismissRequest = { restore.dismissWarning() },
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
                        restore.confirmWarning()?.let { performRestore(it, channelMayExist = true) }
                    }
                ) {
                    Text(
                        if (restoreGuardUnavailable) "Continue Anyway" else "Restore Anyway",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            },
            dismissButton = {
                NeutralTextButton(onClick = { restore.dismissWarning() }) {
                    Text("Cancel")
                }
            },
        )
    }
}
