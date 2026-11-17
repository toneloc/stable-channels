package com.stablechannels.app.ui.settings

import android.widget.Toast
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.fragment.app.FragmentActivity
import com.stablechannels.app.AppState
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import com.stablechannels.app.domain.seed.SeedAccessPolicy
import com.stablechannels.app.services.AppAccessPreferencesManager
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.ui.settings.seed.ObserveSeedLifecyclePhase
import com.stablechannels.app.ui.settings.seed.RestoreSeedSheet
import com.stablechannels.app.ui.settings.seed.RevealSecretPhraseDialog

@Composable
fun BackupView(appState: AppState) {
    val context = LocalContext.current
    val activity = LocalContext.current as? FragmentActivity
    val scope = rememberCoroutineScope()

    var showSeedWords by remember { mutableStateOf(false) }
    val restore = remember { RestoreFlow() }
    // Read after the auth prompt returns, so a reply that lands in background opens nothing.
    var lifecyclePhase by remember { mutableStateOf(AppLifecyclePhase.ACTIVE) }

    // The reveal dialog closes itself on background; the restore sheet holds a typed phrase too.
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

    RestoreSeedSheet(appState, restore)
}
