package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import com.stablechannels.app.domain.seed.SeedAccessPolicy
import com.stablechannels.app.ui.components.DialogEdgeToEdgeEffect

/**
 * Full-screen secure window for the phrase: FLAG_SECURE keeps it out of screenshots and the recents
 * thumbnail, and leaving the app closes it.
 */
@Composable
fun RevealSecretPhraseDialog(mnemonic: String, onDismiss: () -> Unit) {
    ObserveSeedLifecyclePhase { if (SeedAccessPolicy.shouldCloseSeedScreens(it)) onDismiss() }
    Dialog(
        onDismissRequest = onDismiss,
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
            RevealQuizSecretPhrase(
                mnemonic = mnemonic,
                onDone = onDismiss,
                modifier = Modifier.safeDrawingPadding(),
            )
        }
    }
}
