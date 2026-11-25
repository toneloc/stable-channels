package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import com.stablechannels.app.AppState
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.ui.components.DialogEdgeToEdgeEffect
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.ui.settings.RestoreFlow
import com.stablechannels.app.ui.settings.RestoreWarning
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Full-screen secure restore sheet. Call it always, not only while open: its scope then outlives
 * the sheet, as the restore did when it ran from BackupView.
 */
@Composable
internal fun RestoreSeedSheet(appState: AppState, restore: RestoreFlow) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

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

    fun checkAndRestore() {
        val check = restore.startCheck() ?: return
        // Restore guard: a seed-only restore wipes LDK state; if this seed's node still has an
        // LSP channel it will be force-closed at the next reestablish. Detect that and require
        // explicit opt-in; fail-warn if the LSP is unreachable or derivation fails.
        scope.launch(Dispatchers.IO) {
            val nodeId = NodeService.deriveNodeId(context, check.phrase)
            val exists = nodeId?.let { appState.lspChannelExists(it) }
            if (exists == null) {
                AuditService.log(
                    "RESTORE_GUARD_UNAVAILABLE",
                    mapOf("node_id" to (nodeId ?: "derive_failed")),
                )
            }
            withContext(Dispatchers.Main) {
                restore
                    .onChecked(check, exists) { warning ->
                        // Logged only when the warning really opens, never for a stale check; the
                        // write stays off the main thread.
                        if (warning == RestoreWarning.OPEN_CHANNEL) {
                            scope.launch(Dispatchers.IO) {
                                AuditService.log(
                                    "RESTORE_ACTIVE_CHANNEL_DETECTED",
                                    mapOf("node_id" to nodeId),
                                )
                            }
                        }
                    }
                    ?.let { performRestore(it, channelMayExist = false) }
            }
        }
    }

    if (!restore.isOpen) return
    Dialog(
        onDismissRequest = restore::close,
        properties =
            DialogProperties(
                usePlatformDefaultWidth = false,
                dismissOnClickOutside = false,
                decorFitsSystemWindows = false,
                securePolicy = SecureFlagPolicy.SecureOn,
            ),
    ) {
        DialogEdgeToEdgeEffect()
        Surface(Modifier.fillMaxSize(), color = RevealColors.groupedBackground) {
            Column(Modifier.safeDrawingPadding()) {
                TextButton(
                    onClick = restore::close,
                    enabled = !restore.isBusy,
                    modifier = Modifier.padding(horizontal = 4.dp),
                ) {
                    Text("Cancel", fontSize = 17.sp, color = RevealColors.blue)
                }
                RestoreSeedForm(restore, onRestore = ::checkAndRestore, Modifier.weight(1f))
            }
        }
        restore.warning?.let { warning ->
            RestoreWarningDialog(
                warning = warning,
                onConfirm = {
                    restore.confirmWarning()?.let { performRestore(it, channelMayExist = true) }
                },
                onDismiss = restore::dismissWarning,
            )
        }
    }
}

/**
 * Restore guard: explicit opt-in when the seed's node still has an open LSP channel that a
 * seed-only restore would force-close (or when the check could not run).
 */
@Composable
private fun RestoreWarningDialog(
    warning: RestoreWarning,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val restoreGuardUnavailable = warning == RestoreWarning.UNVERIFIED
    AlertDialog(
        onDismissRequest = onDismiss,
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
            NeutralTextButton(onClick = onConfirm) {
                Text(
                    if (restoreGuardUnavailable) "Continue Anyway" else "Restore Anyway",
                    color = MaterialTheme.colorScheme.error,
                )
            }
        },
        dismissButton = { NeutralTextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
